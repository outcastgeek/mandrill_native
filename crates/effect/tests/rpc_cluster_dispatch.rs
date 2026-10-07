#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::cluster::{
    ClusterAddress, ClusterEntityId, ClusterEntityType, ClusterExecutor, ClusterShardId,
    EntityDispatchError, EntityDispatcher, ReplyKind,
};
use effect::encoding::{FrameDecoder, FramePolicy};
use effect::rpc::{
    HasRpcStreamConnector, OperationId, RpcCallId, RpcEnvelope, RpcEnvelopeKind, RpcEnvelopePolicy,
    RpcHandler, RpcOperation, RpcProduced, RpcServerError, RpcStreamClient, RpcStreamConnector,
    RpcStreamError, RpcStreamHandler, RpcStreamItem, RpcStreamOperation, RpcStreamProducer,
    RpcStreamServer, RpcStreamServerError, RpcStreamSession, RpcUnaryServer, RpcVersion,
};
use effect::{
    Effect, Exit, JsonValue, KeyDomain, KeyNamespace, KeySegment, ParseLimits, Scope, StableKey,
    json_i64,
};
use futures_executor::block_on;
use std::collections::{BTreeMap, VecDeque};
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "rpc-cluster-test";
}

#[test]
fn framed_rpc_dispatch_checks_address_before_invoking_typed_handler() {
    let limit = NonZeroUsize::new(512).unwrap();
    let policy = RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: limit,
        },
        json: ParseLimits {
            bytes: limit,
            depth: limit,
            nodes: limit,
            collection: limit,
            issues: limit,
        },
    };
    let operation_id = OperationId::new("increment").unwrap();
    let revision = RpcVersion::new(NonZeroU32::new(1).unwrap());
    let operation = RpcOperation::unary(
        operation_id.clone(),
        revision,
        json_i64(),
        json_i64(),
        json_i64(),
        policy.json,
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&calls);
    let handler = RpcHandler::new(operation, move |request| {
        let witness = Arc::clone(&witness);
        if request < 0 {
            Effect::<i64, i64, ()>::fail_with(move || {
                witness.fetch_add(1, Ordering::SeqCst);
                request
            })
        } else {
            Effect::<i64, i64, ()>::sync_once(move || {
                witness.fetch_add(1, Ordering::SeqCst);
                request + 1
            })
        }
    });
    let namespace = KeyNamespace::<Domain>::new("entities").unwrap();
    let key = |name: &'static str| {
        StableKey::new(&namespace, [KeySegment::Text(Arc::from(name))]).unwrap()
    };
    let expected = key("expected");
    let expected_bytes: Arc<[u8]> = Arc::from(expected.as_bytes());
    let entity_type = ClusterEntityType::new("counter").unwrap();
    let dispatcher = EntityDispatcher::<()>::from_addressed_handler(
        policy,
        entity_type.clone(),
        handler,
        move |address, _| address.entity.as_bytes() == expected_bytes.as_ref(),
    )
    .unwrap();
    let address = ClusterAddress {
        entity_type,
        entity: ClusterEntityId::from_stable(expected),
        shard: ClusterShardId::from_stable(key("shard")),
    };
    let request = |value| {
        RpcEnvelope {
            operation: operation_id.clone(),
            version: revision,
            call: RpcCallId::new("call").unwrap(),
            kind: RpcEnvelopeKind::Request,
            sequence: None,
            payload: JsonValue::from(value),
            metadata: BTreeMap::new(),
        }
        .encode(policy)
        .unwrap()
        .into_bytes()
        .unwrap()
    };

    let mut wrong = address.clone();
    wrong.entity = ClusterEntityId::from_stable(key("wrong"));
    assert!(matches!(
        block_on(
            dispatcher
                .execute(wrong, Arc::from(request(41)))
                .run(Arc::new(()))
        ),
        Exit::Fail(EntityDispatchError::AddressMismatch)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let success = match block_on(
        dispatcher
            .execute(address.clone(), Arc::from(request(41)))
            .run(Arc::new(())),
    ) {
        Exit::Success(reply) => reply,
        _ => panic!("dispatch failed"),
    };
    assert_eq!(success.kind, ReplyKind::Success);
    let mut decoder = FrameDecoder::new(policy.frame);
    let frames = decoder.push(&success.payload).unwrap();
    decoder.finish().unwrap();
    let envelope = RpcEnvelope::decode(&frames[0], policy).unwrap();
    assert_eq!(envelope.kind, RpcEnvelopeKind::Success);
    assert_eq!(envelope.payload, JsonValue::from(42));

    let failure = match block_on(
        dispatcher
            .execute(address.clone(), Arc::from(request(-7)))
            .run(Arc::new(())),
    ) {
        Exit::Success(reply) => reply,
        _ => panic!("expected domain reply"),
    };
    assert_eq!(failure.kind, ReplyKind::DomainError);
    let unknown = RpcEnvelope {
        operation: OperationId::new("missing").unwrap(),
        version: revision,
        call: RpcCallId::new("call").unwrap(),
        kind: RpcEnvelopeKind::Request,
        sequence: None,
        payload: JsonValue::from(41),
        metadata: BTreeMap::new(),
    }
    .encode(policy)
    .unwrap()
    .into_bytes()
    .unwrap();
    assert!(matches!(
        block_on(
            dispatcher
                .execute(address.clone(), Arc::from(unknown))
                .run(Arc::new(()))
        ),
        Exit::Fail(EntityDispatchError::UnknownOperation { .. })
    ));
    assert!(matches!(
        block_on(
            dispatcher
                .execute(address, Arc::from(&b"truncated"[..]))
                .run(Arc::new(()))
        ),
        Exit::Fail(EntityDispatchError::Wire(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

fn campaign_number(value: &JsonValue, path: &[&str]) -> u64 {
    path.iter()
        .try_fold(value, |current, segment| current.get(*segment))
        .and_then(JsonValue::as_u64)
        .expect("configured RPC workload")
}

fn campaign_next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn configured_rpc_unary_histories_preserve_correlation_and_handler_gate() {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/feature_campaigns.json")
        });
    let config: JsonValue = std::fs::read_to_string(configured)
        .expect("campaign configuration is readable")
        .parse()
        .expect("campaign configuration is JSON");
    let seeds = campaign_number(&config, &["campaigns", "rpc", "seeds"]);
    let transitions = campaign_number(&config, &["campaigns", "rpc", "transitions"]);
    let bytes = usize::try_from(campaign_number(&config, &["budgets", "bytes"]))
        .expect("byte budget fits usize");
    let limit = NonZeroUsize::new(bytes).expect("positive byte budget");
    assert!(seeds >= 1_000 && transitions >= 100);
    let policy = RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: limit,
        },
        json: ParseLimits {
            bytes: limit,
            depth: limit,
            nodes: limit,
            collection: limit,
            issues: limit,
        },
    };
    let operation_id = OperationId::new("campaign-rpc-increment").expect("operation identity");
    let version = RpcVersion::new(NonZeroU32::MIN);
    let operation = RpcOperation::unary(
        operation_id.clone(),
        version,
        json_i64(),
        json_i64(),
        json_i64(),
        policy.json,
    );
    let invocations = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&invocations);
    let server = RpcUnaryServer::new(
        RpcHandler::new(operation, move |request| {
            observed.fetch_add(1, Ordering::SeqCst);
            Effect::<i64, i64, ()>::from_result_once(if request < 0 {
                Err(request)
            } else {
                Ok(request + 1)
            })
        }),
        policy,
    );
    let cases = [
        RpcEnvelopeKind::Request,
        RpcEnvelopeKind::Request,
        RpcEnvelopeKind::Request,
        RpcEnvelopeKind::Request,
        RpcEnvelopeKind::Request,
        RpcEnvelopeKind::Request,
        RpcEnvelopeKind::Success,
    ];
    let mut coverage = vec![0_u64; cases.len()];
    let mut handled = 0_usize;
    for seed in 0..seeds {
        let mut state = campaign_number(&config, &["seed"])
            .wrapping_add(seed)
            .max(1);
        for transition in 0..transitions {
            let choice = usize::try_from(campaign_next(&mut state) % cases.len() as u64)
                .expect("case index fits usize");
            coverage[choice] += 1;
            let value = i64::try_from(campaign_next(&mut state) % transitions)
                .expect("configured value fits i64");
            let request = if choice == 1 { -value - 1 } else { value };
            let call = RpcCallId::new(format!("campaign-call-{}", transition % transitions))
                .expect("stable call identity");
            let envelope = RpcEnvelope {
                operation: if choice == 4 {
                    OperationId::new("campaign-other-operation").expect("other operation identity")
                } else {
                    operation_id.clone()
                },
                version: if choice == 5 {
                    RpcVersion::new(
                        NonZeroU32::new(version.value().get() + 1).expect("next version"),
                    )
                } else {
                    version
                },
                call: call.clone(),
                kind: cases[choice],
                sequence: None,
                payload: JsonValue::from(request),
                metadata: BTreeMap::new(),
            };
            let bytes = if choice == 3 {
                Vec::new()
            } else {
                envelope
                    .encode(policy)
                    .expect("valid campaign envelope")
                    .into_bytes()
                    .expect("valid campaign frame")
            };
            let actual = block_on(server.handle(bytes).run(Arc::new(())));
            match choice {
                0 | 1 | 2 => {
                    handled += 1;
                    let frame = match actual {
                        Exit::Success(frame) => frame,
                        _ => panic!("seed {seed}, transition {transition}: valid call rejected"),
                    };
                    let mut decoder = FrameDecoder::new(policy.frame);
                    let encoded = frame.into_bytes().expect("response frame");
                    let frames = decoder.push(&encoded).expect("response decodes");
                    decoder.finish().expect("response complete");
                    assert_eq!(frames.len(), 1);
                    let response =
                        RpcEnvelope::decode(&frames[0], policy).expect("response envelope decodes");
                    assert_eq!(response.call, call);
                    assert_eq!(response.operation, operation_id);
                    assert_eq!(response.version, version);
                    assert_eq!(
                        response.kind,
                        if request < 0 {
                            RpcEnvelopeKind::DomainError
                        } else {
                            RpcEnvelopeKind::Success
                        }
                    );
                    assert_eq!(
                        response.payload,
                        JsonValue::from(if request < 0 { request } else { request + 1 })
                    );
                }
                3 => assert!(matches!(actual, Exit::Fail(RpcServerError::MissingRequest))),
                4 | 5 => assert!(matches!(
                    actual,
                    Exit::Fail(RpcServerError::OperationMismatch)
                )),
                6 => assert!(matches!(
                    actual,
                    Exit::Fail(RpcServerError::UnexpectedRequestKind(
                        RpcEnvelopeKind::Success
                    ))
                )),
                _ => unreachable!("case index came from the case table"),
            }
            assert_eq!(invocations.load(Ordering::SeqCst), handled);
        }
    }
    assert!(coverage.iter().all(|count| *count > 0));
    campaign_report::completed(
        "rpc_unary",
        serde_json::json!({
            "seeds": seeds, "transitions": transitions, "coverage": coverage,
        }),
    );
}

struct ScriptedStreamSession {
    incoming: Arc<Mutex<VecDeque<Option<Vec<u8>>>>>,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
    closes: Arc<AtomicUsize>,
}

impl RpcStreamSession for ScriptedStreamSession {
    type Error = ();

    fn send(&self, frame: effect::encoding::FrameParts) -> Effect<(), Self::Error> {
        let sent = Arc::clone(&self.sent);
        Effect::try_sync_once(
            move || {
                let bytes = frame.into_bytes().map_err(|_| ())?;
                sent.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(bytes);
                Ok(())
            },
            std::convert::identity,
        )
    }

    fn receive_frame(&self) -> Effect<Option<Vec<u8>>, Self::Error> {
        let incoming = Arc::clone(&self.incoming);
        Effect::sync(move || {
            incoming
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_front()
                .flatten()
        })
    }

    fn close(&self) -> Effect<(), Self::Error> {
        let closes = Arc::clone(&self.closes);
        Effect::sync(move || {
            closes.fetch_add(1, Ordering::SeqCst);
        })
    }
}

struct ScriptedStreamConnector(Arc<ScriptedStreamSession>);

impl RpcStreamConnector for ScriptedStreamConnector {
    type Error = ();
    type Session = ScriptedStreamSession;

    fn open(&self) -> Effect<Arc<Self::Session>, Self::Error> {
        let session = Arc::clone(&self.0);
        Effect::sync(move || Arc::clone(&session))
    }
}

struct StreamEnvironment(Arc<ScriptedStreamConnector>);

impl HasRpcStreamConnector for StreamEnvironment {
    type Connector = ScriptedStreamConnector;

    fn rpc_stream_connector(&self) -> Arc<Self::Connector> {
        Arc::clone(&self.0)
    }
}

fn stream_frame(
    operation: OperationId,
    version: RpcVersion,
    call: RpcCallId,
    kind: RpcEnvelopeKind,
    sequence: Option<NonZeroU64>,
    policy: RpcEnvelopePolicy,
) -> Vec<u8> {
    RpcEnvelope {
        operation,
        version,
        call,
        kind,
        sequence,
        payload: JsonValue::from(42),
        metadata: BTreeMap::new(),
    }
    .encode(policy)
    .expect("scripted response envelope")
    .into_bytes()
    .expect("scripted response frame")
}

fn control_frame(
    operation: OperationId,
    version: RpcVersion,
    call: RpcCallId,
    kind: RpcEnvelopeKind,
    sequence: Option<NonZeroU64>,
    policy: RpcEnvelopePolicy,
) -> Vec<u8> {
    RpcEnvelope {
        operation,
        version,
        call,
        kind,
        sequence,
        payload: JsonValue::Null,
        metadata: BTreeMap::new(),
    }
    .encode(policy)
    .expect("scripted control envelope")
    .into_bytes()
    .expect("scripted control frame")
}

#[test]
fn configured_rpc_stream_histories_preserve_acknowledgement_and_scope() {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/feature_campaigns.json")
        });
    let config: JsonValue = std::fs::read_to_string(configured)
        .expect("campaign configuration is readable")
        .parse()
        .expect("campaign configuration is JSON");
    let seeds = campaign_number(&config, &["campaigns", "rpc", "seeds"]);
    let transitions = campaign_number(&config, &["campaigns", "rpc", "transitions"]);
    let limit = NonZeroUsize::new(
        usize::try_from(campaign_number(&config, &["budgets", "bytes"]))
            .expect("byte budget fits usize"),
    )
    .expect("positive byte budget");
    let policy = RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: limit,
        },
        json: ParseLimits {
            bytes: limit,
            depth: limit,
            nodes: limit,
            collection: limit,
            issues: limit,
        },
    };
    let operation = OperationId::new("campaign-rpc-stream").expect("stream operation identity");
    let version = RpcVersion::new(NonZeroU32::MIN);
    let client = Arc::new(RpcStreamClient::<i64, i64, i64, i64, ()>::new(
        RpcStreamOperation::new(
            operation.clone(),
            version,
            json_i64(),
            json_i64(),
            json_i64(),
            json_i64(),
            policy.json,
        ),
        policy,
    ));
    let observed_pulls = Arc::new(AtomicUsize::new(0));
    let pulls = Arc::clone(&observed_pulls);
    let server = RpcStreamServer::new(
        RpcStreamHandler::new(
            RpcStreamOperation::new(
                operation.clone(),
                version,
                json_i64(),
                json_i64(),
                json_i64(),
                json_i64(),
                policy.json,
            ),
            move |request, _| {
                let pulls = Arc::clone(&pulls);
                Effect::sync_once(move || {
                    let local = Arc::new(AtomicUsize::new(0));
                    RpcStreamProducer::new(move || {
                        let local = Arc::clone(&local);
                        let pulls = Arc::clone(&pulls);
                        Effect::<RpcProduced<i64, i64>, i64>::sync(move || {
                            pulls.fetch_add(1, Ordering::SeqCst);
                            if local.fetch_add(1, Ordering::SeqCst) == 0 {
                                RpcProduced::Chunk(request + 1)
                            } else {
                                RpcProduced::Terminal(request + 2)
                            }
                        })
                    })
                })
            },
        ),
        policy,
    );
    let scenarios = [
        "ordered",
        "reordered",
        "ack-required",
        "duplicate",
        "disconnect",
        "wrong-call",
        "cancel",
    ];
    let mut coverage = vec![0_u64; scenarios.len()];
    for seed in 0..seeds {
        let mut state = campaign_number(&config, &["seed"])
            .wrapping_add(seed)
            .max(1);
        for transition in 0..transitions {
            let choice = usize::try_from(campaign_next(&mut state) % scenarios.len() as u64)
                .expect("scenario index fits usize");
            coverage[choice] += 1;
            let call = RpcCallId::new(format!("stream-{seed}-{transition}"))
                .expect("stream call identity");
            let chunk = |sequence, call: RpcCallId| {
                stream_frame(
                    operation.clone(),
                    version,
                    call,
                    RpcEnvelopeKind::StreamChunk,
                    Some(sequence),
                    policy,
                )
            };
            let first = NonZeroU64::MIN;
            let second = NonZeroU64::new(first.get() + 1).expect("next sequence");
            let terminal = || {
                stream_frame(
                    operation.clone(),
                    version,
                    call.clone(),
                    RpcEnvelopeKind::StreamEnd,
                    None,
                    policy,
                )
            };
            let incoming = match choice {
                0 | 2 => VecDeque::from([Some(chunk(first, call.clone())), Some(terminal())]),
                1 => VecDeque::from([Some(chunk(second, call.clone()))]),
                3 => VecDeque::from([
                    Some(chunk(first, call.clone())),
                    Some(chunk(first, call.clone())),
                ]),
                4 => VecDeque::from([None]),
                5 => VecDeque::from([Some(chunk(
                    first,
                    RpcCallId::new("other-call").expect("other call identity"),
                ))]),
                6 => VecDeque::new(),
                _ => unreachable!("scenario index came from the scenario table"),
            };
            let session = Arc::new(ScriptedStreamSession {
                incoming: Arc::new(Mutex::new(incoming)),
                sent: Arc::new(Mutex::new(Vec::new())),
                closes: Arc::new(AtomicUsize::new(0)),
            });
            let environment = Arc::new(StreamEnvironment(Arc::new(ScriptedStreamConnector(
                Arc::clone(&session),
            ))));
            let client = Arc::clone(&client);
            let observed_call = call.clone();
            let result = block_on(
                Scope::<RpcStreamError<i64, ()>>::scoped(move |scope| {
                    let client = Arc::clone(&client);
                    let call = call.clone();
                    client
                        .open(&scope, 0, call, BTreeMap::new())
                        .flat_map(move |reader| match choice {
                            0 | 2 => {
                                let after_chunk = reader.clone();
                                reader.next::<StreamEnvironment>().flat_map(move |item| {
                                    let valid_chunk = matches!(item, RpcStreamItem::Chunk {
                                        sequence, value
                                    } if sequence == first && value == 42);
                                    let after_ack = after_chunk.clone();
                                    let check_ack = if choice == 2 {
                                        after_chunk.next::<StreamEnvironment>().either().map(
                                            move |result| {
                                                matches!(result, Err(
                                                RpcStreamError::AcknowledgementRequired(sequence)
                                            ) if sequence == first)
                                            },
                                        )
                                    } else {
                                        Effect::sync(|| true)
                                    };
                                    check_ack.flat_map(move |blocked| {
                                        let after_ack = after_ack.clone();
                                        after_ack.acknowledge::<StreamEnvironment>(first).flat_map(
                                            move |_| {
                                                after_ack.next::<StreamEnvironment>().map(
                                                    move |item| {
                                                        valid_chunk
                                                            && blocked
                                                            && matches!(
                                                                item,
                                                                RpcStreamItem::Terminal(42)
                                                            )
                                                    },
                                                )
                                            },
                                        )
                                    })
                                })
                            }
                            1 => reader
                                .next::<StreamEnvironment>()
                                .either()
                                .map(move |result| {
                                    matches!(result, Err(RpcStreamError::SequenceMismatch {
                                    expected, actual
                                }) if expected == first && actual == second)
                                }),
                            3 => {
                                let after_chunk = reader.clone();
                                reader.next::<StreamEnvironment>().flat_map(move |_| {
                                    let after_ack = after_chunk.clone();
                                    after_chunk
                                        .acknowledge::<StreamEnvironment>(first)
                                        .flat_map(move |_| {
                                            after_ack.next::<StreamEnvironment>().either().map(
                                                move |result| {
                                                    matches!(result, Err(
                                                    RpcStreamError::SequenceMismatch {
                                                        expected, actual
                                                    }
                                                ) if expected == second && actual == first)
                                                },
                                            )
                                        })
                                })
                            }
                            4 => reader
                                .next::<StreamEnvironment>()
                                .either()
                                .map(|result| matches!(result, Err(RpcStreamError::Disconnected))),
                            5 => reader.next::<StreamEnvironment>().either().map(|result| {
                                matches!(result, Err(RpcStreamError::CorrelationMismatch))
                            }),
                            6 => reader.cancel::<StreamEnvironment>().map(|_| true),
                            _ => unreachable!("scenario index came from the scenario table"),
                        })
                })
                .run(environment),
            );
            assert!(
                matches!(result, Exit::Success(true)),
                "{scenarios:?}, seed {seed}, transition {transition}"
            );
            assert_eq!(session.closes.load(Ordering::SeqCst), 1);
            let sent = session
                .sent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(
                sent.len(),
                if matches!(choice, 0 | 2 | 3 | 6) {
                    2
                } else {
                    1
                }
            );
            for (index, bytes) in sent.iter().enumerate() {
                let mut decoder = FrameDecoder::new(policy.frame);
                let frames = decoder.push(bytes).expect("outbound frame decodes");
                decoder.finish().expect("outbound frame complete");
                assert_eq!(frames.len(), 1);
                let message =
                    RpcEnvelope::decode(&frames[0], policy).expect("outbound envelope decodes");
                assert_eq!(message.call, observed_call);
                assert_eq!(message.operation, operation);
                assert_eq!(message.version, version);
                assert_eq!(
                    message.kind,
                    if index == 0 {
                        RpcEnvelopeKind::Request
                    } else if choice == 6 {
                        RpcEnvelopeKind::Cancel
                    } else {
                        RpcEnvelopeKind::Acknowledge
                    }
                );
                assert_eq!(
                    message.sequence,
                    if index == 1 && choice != 6 {
                        Some(first)
                    } else {
                        None
                    }
                );
            }
            drop(sent);

            let request_operation = if choice == 6 {
                OperationId::new("other-stream-operation").expect("other stream operation")
            } else {
                operation.clone()
            };
            let request = if choice == 5 {
                Vec::new()
            } else {
                stream_frame(
                    request_operation,
                    version,
                    observed_call.clone(),
                    RpcEnvelopeKind::Request,
                    None,
                    policy,
                )
            };
            let control = match choice {
                0 => Some(control_frame(
                    operation.clone(),
                    version,
                    observed_call.clone(),
                    RpcEnvelopeKind::Acknowledge,
                    Some(first),
                    policy,
                )),
                1 => Some(control_frame(
                    operation.clone(),
                    version,
                    observed_call.clone(),
                    RpcEnvelopeKind::Acknowledge,
                    Some(second),
                    policy,
                )),
                2 => Some(control_frame(
                    operation.clone(),
                    version,
                    observed_call.clone(),
                    RpcEnvelopeKind::Cancel,
                    None,
                    policy,
                )),
                4 => Some(control_frame(
                    operation.clone(),
                    version,
                    RpcCallId::new("other-control-call").expect("other control call"),
                    RpcEnvelopeKind::Acknowledge,
                    Some(first),
                    policy,
                )),
                3 | 5 | 6 => None,
                _ => unreachable!("scenario index came from the scenario table"),
            };
            let server_session = Arc::new(ScriptedStreamSession {
                incoming: Arc::new(Mutex::new(VecDeque::from([Some(request), control]))),
                sent: Arc::new(Mutex::new(Vec::new())),
                closes: Arc::new(AtomicUsize::new(0)),
            });
            let before_pulls = observed_pulls.load(Ordering::SeqCst);
            let served = block_on(server.serve(Arc::clone(&server_session)).run(Arc::new(())));
            match choice {
                0 => assert!(matches!(served, Exit::Success(()))),
                1 => assert!(matches!(served, Exit::Fail(
                    RpcStreamServerError::SequenceMismatch { expected, actual }
                ) if expected == first && actual == second)),
                2 => assert!(matches!(
                    served,
                    Exit::Fail(RpcStreamServerError::Cancelled)
                )),
                3 => assert!(matches!(
                    served,
                    Exit::Fail(RpcStreamServerError::Disconnected)
                )),
                4 => assert!(matches!(
                    served,
                    Exit::Fail(RpcStreamServerError::CorrelationMismatch)
                )),
                5 => assert!(matches!(served, Exit::Fail(RpcStreamServerError::Wire(_)))),
                6 => assert!(matches!(
                    served,
                    Exit::Fail(RpcStreamServerError::OperationMismatch)
                )),
                _ => unreachable!("scenario index came from the scenario table"),
            }
            assert_eq!(server_session.closes.load(Ordering::SeqCst), 1);
            assert_eq!(
                observed_pulls.load(Ordering::SeqCst) - before_pulls,
                if choice == 0 {
                    2
                } else if choice <= 4 {
                    1
                } else {
                    0
                }
            );
            let server_sent = server_session
                .sent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(
                server_sent.len(),
                if choice == 0 {
                    2
                } else if choice <= 4 {
                    1
                } else {
                    0
                }
            );
            for (index, bytes) in server_sent.iter().enumerate() {
                let mut decoder = FrameDecoder::new(policy.frame);
                let frames = decoder.push(bytes).expect("server frame decodes");
                decoder.finish().expect("server frame complete");
                assert_eq!(frames.len(), 1);
                let message =
                    RpcEnvelope::decode(&frames[0], policy).expect("server envelope decodes");
                assert_eq!(message.operation, operation);
                assert_eq!(message.call, observed_call);
                assert_eq!(message.version, version);
                assert_eq!(
                    message.kind,
                    if index == 0 {
                        RpcEnvelopeKind::StreamChunk
                    } else {
                        RpcEnvelopeKind::StreamEnd
                    }
                );
                assert_eq!(
                    message.sequence,
                    if index == 0 { Some(first) } else { None }
                );
                assert_eq!(
                    message.payload,
                    JsonValue::from(if index == 0 { 43 } else { 44 })
                );
            }
        }
    }
    assert!(coverage.iter().all(|count| *count > 0));
    campaign_report::completed(
        "rpc_stream",
        serde_json::json!({
            "seeds": seeds, "transitions": transitions, "coverage": coverage,
        }),
    );
}

#[path = "support/drive.rs"]
mod correction_drive;
#[test]
fn interrupted_rpc_acknowledgement_closes_once_and_preserves_close_failure() {
    use effect::{Cancellation, CleanupFailure, HasCancellation, PrimaryOutcome};
    struct Session {
        inner: ScriptedStreamSession,
        sends: AtomicUsize,
        entered: Arc<AtomicUsize>,
    }
    impl RpcStreamSession for Session {
        type Error = ();
        fn send(&self, frame: effect::encoding::FrameParts) -> Effect<(), ()> {
            if self.sends.fetch_add(1, Ordering::SeqCst) == 1 {
                let entered = Arc::clone(&self.entered);
                Effect::from_future(move |cancel| {
                    entered.store(1, Ordering::SeqCst);
                    async move {
                        cancel.cancelled().await;
                    }
                })
            } else {
                self.inner.send(frame)
            }
        }
        fn receive_frame(&self) -> Effect<Option<Vec<u8>>, ()> {
            self.inner.receive_frame()
        }
        fn close(&self) -> Effect<(), ()> {
            let closes = Arc::clone(&self.inner.closes);
            Effect::sync(move || {
                closes.fetch_add(1, Ordering::SeqCst);
            })
            .flat_map(|_| Effect::fail(()))
        }
    }
    struct Connector(Arc<Session>);
    impl RpcStreamConnector for Connector {
        type Error = ();
        type Session = Session;
        fn open(&self) -> Effect<Arc<Session>, ()> {
            let session = Arc::clone(&self.0);
            Effect::sync(move || Arc::clone(&session))
        }
    }
    struct Env {
        connector: Arc<Connector>,
        cancel: Cancellation,
    }
    impl HasRpcStreamConnector for Env {
        type Connector = Connector;
        fn rpc_stream_connector(&self) -> Arc<Connector> {
            Arc::clone(&self.connector)
        }
    }
    impl HasCancellation for Env {
        fn cancellation(&self) -> &Cancellation {
            &self.cancel
        }
    }
    let limit = NonZeroUsize::new(1024).unwrap();
    let policy = RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: limit,
        },
        json: ParseLimits {
            bytes: limit,
            depth: limit,
            nodes: limit,
            collection: limit,
            issues: limit,
        },
    };
    let operation = OperationId::new("interrupted-ack").unwrap();
    let version = RpcVersion::new(NonZeroU32::MIN);
    let call = RpcCallId::new("ack").unwrap();
    let first = NonZeroU64::MIN;
    let closes = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicUsize::new(0));
    let incoming = VecDeque::from([Some(stream_frame(
        operation.clone(),
        version,
        call.clone(),
        RpcEnvelopeKind::StreamChunk,
        Some(first),
        policy,
    ))]);
    let session = Arc::new(Session {
        inner: ScriptedStreamSession {
            incoming: Arc::new(Mutex::new(incoming)),
            sent: Arc::new(Mutex::new(Vec::new())),
            closes: Arc::clone(&closes),
        },
        sends: AtomicUsize::new(0),
        entered: Arc::clone(&entered),
    });
    let env = Arc::new(Env {
        connector: Arc::new(Connector(session)),
        cancel: Cancellation::new(),
    });
    let client = Arc::new(RpcStreamClient::<i64, i64, i64, i64, ()>::new(
        RpcStreamOperation::new(
            operation,
            version,
            json_i64(),
            json_i64(),
            json_i64(),
            json_i64(),
            policy.json,
        ),
        policy,
    ));
    let program: Effect<(), RpcStreamError<i64, ()>, Env> = Scope::scoped(move |scope| {
        let client = Arc::clone(&client);
        client
            .open(&scope, 0, call.clone(), BTreeMap::new())
            .flat_map(move |reader| {
                let acknowledge = reader.clone();
                reader
                    .next::<Env>()
                    .flat_map(move |_| acknowledge.acknowledge::<Env>(first))
            })
    });
    let mut future = Box::pin(program.run_cancellable(Arc::clone(&env)));
    correction_drive::until(future.as_mut(), || entered.load(Ordering::SeqCst) == 1);
    assert!(env.cancel.request());
    let outcome = block_on(future);
    assert!(
        matches!(outcome,Exit::FinalizationFailed(ref details) if matches!(&details.primary,PrimaryOutcome::WasInterrupted)&&matches!(&details.first,CleanupFailure::ReleaseFailure(RpcStreamError::Transport(()))))
    );
    assert_eq!(closes.load(Ordering::SeqCst), 1);
}
