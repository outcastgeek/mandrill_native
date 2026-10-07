use effect::*;
use futures_executor::block_on;
use std::{
    collections::BTreeMap,
    num::{NonZeroU64, NonZeroUsize},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}
fn policy() -> ParsePolicy {
    ParsePolicy {
        limits: ParseLimits {
            bytes: nz(4096),
            depth: nz(32),
            nodes: nz(256),
            collection: nz(32),
            issues: nz(16),
        },
        errors: SchemaErrorMode::Accumulate,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    }
}
fn run<A: Send + 'static, E: Send + 'static>(effect: Effect<A, E>) -> Exit<A, E> {
    block_on(effect.run(Arc::new(())))
}
fn success<A, E>(exit: Exit<A, E>) -> A {
    match exit {
        Exit::Success(value) => value,
        _ => panic!("expected success"),
    }
}
#[test]
fn native_defaults_are_owned_validated_once_and_absence_only() {
    struct Record {
        value: i64,
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&calls);
    let field = json_i64()
        .refine("positive", |n| *n > 0)
        .into_effect::<(), ()>();
    let codec=schema!{struct Record { #[schema(default_with = move ||{let calls=Arc::clone(&calls);Effect::sync(move ||{calls.fetch_add(1,Ordering::SeqCst);7i64})})] value:field } policy:policy(),representation:JsonRepresentation,unknown_fields:UnknownFields::Reject}.unwrap();
    assert_eq!(
        success(run(codec.decode_once(JsonValue::Object(Default::default())))).value,
        7
    );
    assert_eq!(witness.load(Ordering::SeqCst), 1);
    assert!(matches!(
        run(codec.decode_once(JsonValue::Object(
            BTreeMap::from([("value".to_owned(), JsonValue::Null)])
                .into_iter()
                .collect()
        ))),
        Exit::Fail(_)
    ));
    assert_eq!(witness.load(Ordering::SeqCst), 1);
    let bytes = success(run(codec.write_once(
        Record { value: 9 },
        JsonWriterPolicy {
            bytes: nz(64),
            depth: nz(4),
        },
    )));
    assert_eq!(std::str::from_utf8(&bytes).unwrap(), "{\"value\":9}");
}
#[test]
fn configured_native_budgets_are_fresh_and_directional() {
    let codec = native_array(
        json_i64().into_effect::<(), ()>(),
        JsonRepresentation,
        policy(),
    );
    let configured = codec
        .configured(SchemaPolicy {
            parse: policy(),
            json_depth: nz(8),
            string_length: nz(64),
            integer_digits: nz(16),
            pattern_steps: nz(4096),
        })
        .unwrap();
    assert_eq!(success(run(configured.parse("[1,2]"))), vec![1, 2]);
    assert_eq!(success(run(configured.parse("[3,4]"))), vec![3, 4]);
    assert_eq!(
        success(run(configured.validate_once(vec![5, 6]))),
        vec![5, 6]
    );
    assert_eq!(
        success(run(configured.encode_once(vec![7, 8]))),
        JsonValue::Array(vec![7.into(), 8.into()])
    );
    let mut bounded = policy();
    bounded.limits.nodes = nz(4);
    let configured = codec
        .configured(SchemaPolicy {
            parse: bounded,
            json_depth: nz(8),
            string_length: nz(64),
            integer_digits: nz(16),
            pattern_steps: nz(4096),
        })
        .unwrap();
    assert!(matches!(
        run(configured.validate_once(vec![1, 2, 3, 4, 5])),
        Exit::Fail(_)
    ));
    assert!(json_i64().parse_utf8(&[0xff], policy()).is_err());
}
#[test]
fn domain_composites_preserve_nonclone_errors_and_owned_values() {
    #[derive(Debug)]
    struct Error(&'static str);
    let domain = DomainCodec::from_schema(
        json_i64().into_effect::<(), ()>(),
        |_| Error("decode"),
        |_| Error("encode"),
    );
    assert!(matches!(
        run(domain
            .clone()
            .sequence(
                JsonRepresentation,
                policy().limits,
                |_| Error("decode-boundary"),
                |_| Error("encode-boundary")
            )
            .decode_once(JsonValue::Null)),
        Exit::Fail(Error("decode-boundary"))
    ));
    assert_eq!(
        success(run(domain.clone().optional().decode_once(None))),
        None
    );
    assert_eq!(
        success(run(domain
            .clone()
            .nullable(|value| value.is_null(), || JsonValue::Null)
            .encode_once(None))),
        JsonValue::Null
    );
    let map = domain.map(
        JsonRepresentation,
        policy().limits,
        |_| Error("decode-map"),
        |_| Error("encode-map"),
    );
    assert_eq!(
        success(run(map.decode_once(JsonValue::Object(
            [("a".into(), 3.into())].into_iter().collect()
        )))),
        BTreeMap::from([("a".to_owned(), 3)])
    );
}
#[test]
fn exact_numbers_do_not_round_through_floating_point() {
    let n = |text| JsonExactNumber::parse(text, nz(64)).unwrap();
    assert!(
        n("9007199254740993")
            .compare(&n("9007199254740992"))
            .is_gt()
    );
    assert_eq!(n("1.20"), n("12e-1"));
    assert!(n("0.3").multiple_of(&n("0.1")).unwrap());
    assert!(!n("0.31").multiple_of(&n("0.1")).unwrap());
    assert!(n("1e999999").to_big_integer(nz(8)).is_err());
    let codec = json_exact_number(vec![NumericConstraint::MultipleOf(n("0.1"))], nz(64)).unwrap();
    let bytes = codec
        .write(
            &n("0.3"),
            JsonWriterPolicy {
                bytes: nz(64),
                depth: nz(8),
            },
        )
        .unwrap();
    assert_eq!(
        codec
            .parse_str(std::str::from_utf8(&bytes).unwrap(), policy())
            .unwrap(),
        n("0.3")
    );
}
#[test]
fn strings_regex_enums_and_alternatives_preserve_constraints() {
    let codec = json_string_constraints(StringConstraints {
        minimum: 1,
        maximum: 2,
        pattern: None,
        uuid_v4: false,
    })
    .unwrap();
    assert!(codec.decode_ref(&JsonValue::String("é🦀".into())).is_ok());
    assert!(codec.decode_ref(&JsonValue::String("abc".into())).is_err());
    assert!(
        CheckedPattern::new(
            "(a)\\1",
            PatternPolicy {
                compiled_bytes: nz(16384),
                syntax_depth: nz(16),
                steps: nz(4096)
            }
        )
        .is_err()
    );
    let bounded = CheckedPattern::new(
        "a+",
        PatternPolicy {
            compiled_bytes: nz(16384),
            syntax_depth: nz(16),
            steps: nz(1),
        },
    )
    .unwrap();
    assert!(matches!(
        bounded.matches("aaaa").unwrap_err().kind(),
        SchemaIssueKind::PatternBudget
    ));
    let enumeration = json_literal_enum(
        vec![
            JsonValue::String("red".into()),
            JsonValue::String("blue".into()),
        ],
        UnknownLiteralPolicy::Preserve,
    )
    .unwrap();
    assert!(matches!(
        enumeration.decode_ref(&JsonValue::String("green".into())),
        Ok(LiteralEnumValue::Unknown(_))
    ));
    assert!(
        enumeration
            .encode_ref(&LiteralEnumValue::Unknown(JsonValue::Null))
            .is_err()
    );
    let ambiguous = json_one_of(vec![json_i64(), json_i64()], |_| 0, policy()).unwrap();
    assert!(
        matches!(ambiguous.decode_ref(&1.into()).unwrap_err().kind(),SchemaIssueKind::AmbiguousAlternatives{matches} if matches==&vec![0,1])
    );
    let any = json_any_of(vec![json_i64(), json_i64()], policy()).unwrap();
    assert_eq!(any.decode_ref(&1.into()).unwrap().matches().len(), 2);
}
#[test]
fn prefix_absence_is_distinct_from_null_and_array_uniqueness_is_checked() {
    let codec = json_prefix_rest(
        vec![PrefixElement {
            codec: json_nullable(json_i64()),
            required: false,
        }],
        None::<PureCodec<i64, JsonValue>>,
        policy(),
    )
    .unwrap();
    assert!(codec.decode_ref(&JsonValue::Array(vec![])).unwrap().prefix[0].is_none());
    assert_eq!(
        codec
            .decode_ref(&JsonValue::Array(vec![JsonValue::Null]))
            .unwrap()
            .prefix[0],
        Some(None)
    );
    assert!(
        codec
            .decode_ref(&JsonValue::Array(vec![1.into(), 2.into()]))
            .is_err()
    );
    let unique = json_array_constraints(
        json_i64(),
        ArrayConstraints {
            minimum: 1,
            maximum: 4,
            unique: true,
        },
        policy(),
    )
    .unwrap();
    assert!(
        unique
            .decode_ref(&JsonValue::Array(vec![1.into(), 1.into()]))
            .is_err()
    );
}
#[test]
fn writer_failure_never_replays_the_encoder_and_integer_bytes_are_exact() {
    let encodes = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&encodes);
    let codec = PureCodec::with_ast(
        SchemaAst::opaque(),
        |_: &JsonValue| Ok(1i64),
        move |_: &i64| {
            encodes.fetch_add(1, Ordering::SeqCst);
            Ok(JsonValue::Null)
        },
    )
    .with_writer(|_, writer| {
        writer.begin_array()?;
        Err(SchemaIssue::new(SchemaIssueKind::OutOfRange))
    });
    assert!(
        codec
            .write(
                &1,
                JsonWriterPolicy {
                    bytes: nz(64),
                    depth: nz(8)
                }
            )
            .is_err()
    );
    assert_eq!(witness.load(Ordering::SeqCst), 0);
    assert_eq!(
        json_i64()
            .write(
                &i64::MIN,
                JsonWriterPolicy {
                    bytes: nz(64),
                    depth: nz(8)
                }
            )
            .unwrap(),
        i64::MIN.to_string().into_bytes()
    );
    assert_eq!(
        json_u64()
            .write(
                &u64::MAX,
                JsonWriterPolicy {
                    bytes: nz(64),
                    depth: nz(8)
                }
            )
            .unwrap(),
        u64::MAX.to_string().into_bytes()
    );
    assert!(
        json_i64()
            .write(
                &123,
                JsonWriterPolicy {
                    bytes: nz(2),
                    depth: nz(8)
                }
            )
            .is_err()
    );
    assert!(
        json_array(json_array(json_i64(), None), None)
            .write(
                &vec![vec![1]],
                JsonWriterPolicy {
                    bytes: nz(64),
                    depth: nz(1)
                }
            )
            .is_err()
    );
}
#[test]
fn binary_adapter_reuses_the_existing_format_and_owned_channels() {
    use effect::encoding::*;
    let pure = json_i64();
    let binary = SchemaBinary::new(
        pure.ast().clone(),
        BinaryIdentity::new("reference-number", NonZeroU64::MIN).unwrap(),
        BinaryLimits {
            bytes: nz(4096),
            depth: nz(32),
            nodes: nz(256),
            collection: nz(32),
            steps: nz(256),
        },
    );
    let codec = binary
        .effect_codec(pure.clone().into_effect::<(), ()>())
        .unwrap();
    let bytes = success(run(codec.encode_once(123)));
    assert_eq!(bytes, binary.encode_typed(&pure, &123).unwrap());
    assert_eq!(success(run(codec.decode_once(bytes))), 123);
}
#[test]
fn callback_publication_reports_full_and_closed_without_an_executor() {
    let hub = CallbackPubSub::new(CallbackPubSubOptions {
        capacity: nz(1),
        overflow: CallbackOverflow::DropNewest,
        replay: ReplayPolicy::None,
        contention: CallbackContention::Reject,
    })
    .unwrap();
    let pub_hub = hub.clone();
    let program: Effect<(), PubSubError> = Scope::scoped(move |scope| {
        let hub = pub_hub.clone();
        hub.subscribe_scoped(&scope).flat_map(move |_| {
            assert!(matches!(
                hub.try_publish(1),
                Ok(CallbackPublish::Accepted(_))
            ));
            assert!(matches!(
                hub.try_publish(2),
                Ok(CallbackPublish::Dropped(_))
            ));
            hub.shutdown(PubSubShutdown::Discard).map(|_| ())
        })
    });
    success(run(program));
    assert_eq!(hub.try_publish(3).unwrap(), CallbackPublish::Closed);
}
#[test]
fn effectful_stream_acquires_fresh_sources_and_awaits_disposal() {
    let acquired = Arc::new(AtomicUsize::new(0));
    let disposed = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&acquired);
    let cleanup = Arc::clone(&disposed);
    let stream: Stream<i64, (), ()> = Stream::from_async_iterable_effect(
        move || {
            let acquired = Arc::clone(&acquired);
            Effect::sync(move || {
                acquired.fetch_add(1, Ordering::SeqCst);
                futures_util::stream::iter(vec![Ok::<_, ()>(1), Ok(2)])
            })
        },
        std::convert::identity,
        move |source| {
            let disposed = Arc::clone(&disposed);
            Release::sync(move || {
                drop(source);
                disposed.fetch_add(1, Ordering::SeqCst);
            })
        },
    );
    assert_eq!(success(run(stream.run_collect(Some(2)))), vec![1, 2]);
    assert_eq!(success(run(stream.run_collect(Some(2)))), vec![1, 2]);
    assert_eq!(witness.load(Ordering::SeqCst), 2);
    assert_eq!(cleanup.load(Ordering::SeqCst), 2);
}
struct TestDomain;
impl KeyDomain for TestDomain {
    const TAG: &'static str = "reference-update-test";
}
fn key(name: &'static str) -> StableKey<TestDomain> {
    StableKey::new(
        &KeyNamespace::new("reference-update").unwrap(),
        [KeySegment::Text(name.into())],
    )
    .unwrap()
}
#[test]
fn stale_workflow_recovery_cannot_reset_a_new_execution() {
    use effect::workflow::*;
    use std::num::NonZeroU32;
    let engine = MemoryWorkflowEngine::new();
    let definition = WorkflowDefinitionId::new("reference-update").unwrap();
    let version = WorkflowVersion::new(NonZeroU32::MIN);
    success(run(engine.register(WorkflowRegistration {
        id: definition.clone(),
        version,
        compatibility: Arc::from(&b"reference"[..]),
    })));
    let handle = WorkflowHandle {
        run: WorkflowRunId::from_stable(key("run")),
        definition,
        version,
    };
    let ready = success(run(engine.start(handle.clone(), Arc::from(&b"input"[..]))));
    let first = success(run(engine.transition(
        handle.run.clone(),
        ready.checkpoint(),
        WorkflowStatus::Running,
    )));
    let ready = success(run(engine.transition(
        handle.run.clone(),
        first.checkpoint(),
        WorkflowStatus::Ready,
    )));
    let second = success(run(engine.transition(
        handle.run.clone(),
        ready.checkpoint(),
        WorkflowStatus::Running,
    )));
    assert!(second.execution > first.execution);
    assert!(matches!(
        run(engine.transition(
            handle.run.clone(),
            first.checkpoint(),
            WorkflowStatus::Ready
        )),
        Exit::Fail(WorkflowEngineError::StateConflict)
    ));
    assert_eq!(
        success(run(engine.inspect(handle.run)))
            .unwrap()
            .checkpoint(),
        second.checkpoint()
    );
}
#[test]
fn denied_replica_identity_never_reads_a_cursor_or_journal() {
    use effect::eventlog::*;
    struct Env {
        calls: AtomicUsize,
        journal: Arc<MemoryEventJournal>,
        cursor: Arc<MemoryReplicaCursorStore>,
    }
    impl HasEventJournal for Env {
        type Journal = MemoryEventJournal;
        fn event_journal(&self) -> Arc<Self::Journal> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Arc::clone(&self.journal)
        }
    }
    impl HasReplicaCursorStore for Env {
        type Store = MemoryReplicaCursorStore;
        fn replica_cursor_store(&self) -> Arc<Self::Store> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Arc::clone(&self.cursor)
        }
    }
    let stream = JournalStream {
        journal: EventJournalId::from_stable(key("journal")),
        aggregate: AggregateId::from_stable(key("aggregate")),
    };
    let source = ReplicaId::from_stable(key("source"));
    let denied = ReplicaImportPermit::authorize(source.clone(), stream.clone(), |_, _| {
        Err::<(), _>("denied")
    });
    assert!(matches!(denied, Err("denied")));
    let permit = ReplicaImportPermit::authorize(
        ReplicaId::from_stable(key("other")),
        stream.clone(),
        |_, _| Ok::<(), ()>(()),
    )
    .unwrap();
    let batch =
        ReplicaBatch::new(source, stream, JournalCursor::new(0), Arc::from([]), nz(1)).unwrap();
    let env = Arc::new(Env {
        calls: AtomicUsize::new(0),
        journal: Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
            streams: nz(1),
            entries_per_stream: nz(1),
            tombstones_per_stream: nz(1),
        })),
        cursor: Arc::new(MemoryReplicaCursorStore::new(nz(1))),
    });
    assert!(matches!(
        block_on(EventReplicator::import_and_ack::<Env>(batch, permit).run(Arc::clone(&env))),
        Exit::Fail(ReplicaSyncError::Import(ReplicaImportError::Unauthorized))
    ));
    assert_eq!(env.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn repeated_local_hour_is_sorted_by_instant() {
    struct Fold;
    impl CalendarZone for Fold {
        type Error = CalendarTimeError;
        fn resolve(&self, local: CivilDateTime) -> Result<LocalResolution, Self::Error> {
            let base = FixedOffsetZone::east_seconds(0)?.resolve(local)?;
            match (base, local.components()) {
                (LocalResolution::Unique(instant), (1970, 1, 1, 1, _, _)) => {
                    Ok(LocalResolution::Overlap {
                        earlier: instant,
                        later: UtcMillis::new(instant.value() + 3_600_000),
                    })
                }
                (other, _) => Ok(other),
            }
        }
    }
    let cron = Cron::parse("0 15,45 1 * * *", DayMatch::Both).unwrap();
    let occurrences = cron
        .occurrences_between(
            UtcMillis::new(0),
            UtcMillis::new(10_800_000),
            &Fold,
            CalendarTransitionPolicy {
                gap: GapPolicy::Skip,
                overlap: OverlapPolicy::Both,
            },
            CronSearchBudget {
                days: nz(4),
                candidates: nz(32),
            },
        )
        .unwrap();
    assert_eq!(
        occurrences
            .iter()
            .map(|item| item.instant.value())
            .collect::<Vec<_>>(),
        vec![4_500_000, 6_300_000, 8_100_000, 9_900_000]
    );
}

#[test]
fn numeric_literal_enums_reject_equivalent_spellings_and_write_without_encoding() {
    assert!(
        json_literal_enum(
            vec!["1".parse().unwrap(), "1.0".parse().unwrap()],
            UnknownLiteralPolicy::Reject
        )
        .is_err()
    );
    let codec =
        json_literal_enum(vec!["1.0".parse().unwrap()], UnknownLiteralPolicy::Preserve).unwrap();
    assert_eq!(
        codec.decode_ref(&"1".parse().unwrap()).unwrap(),
        LiteralEnumValue::Known(0)
    );
    let bytes = codec
        .write(
            &LiteralEnumValue::Known(0),
            JsonWriterPolicy {
                bytes: nz(32),
                depth: nz(4),
            },
        )
        .unwrap();
    assert_eq!(
        codec
            .decode_ref(&serde_json::from_slice(&bytes).unwrap())
            .unwrap(),
        LiteralEnumValue::Known(0)
    );
}

#[test]
fn native_accumulation_retains_field_paths_and_defaults_require_independent_validation() {
    struct Record {
        first: i64,
        second: i64,
    }
    let codec = schema! { struct Record { first: json_i64().into_effect::<(), ()>(), second: json_i64().into_effect::<(), ()>() } policy: policy(), representation: JsonRepresentation, unknown_fields: UnknownFields::Reject }.unwrap();
    let issue = match run(codec.decode_once(serde_json::json!({"first":"bad","second":"bad"}))) {
        Exit::Fail(issue) => issue,
        _ => panic!("two invalid fields must fail"),
    };
    assert_eq!(issue.children().len(), 2);
    assert!(
        issue
            .children()
            .iter()
            .all(|child| matches!(child.path().first(), Some(IssueSegment::Field(_))))
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let encodes = Arc::clone(&calls);
    let custom = PureCodec::new(
        |value: &JsonValue| {
            value
                .as_i64()
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange))
        },
        move |value: &i64| {
            encodes.fetch_add(1, Ordering::SeqCst);
            Ok(JsonValue::from(*value))
        },
    )
    .refine("positive", |value| *value > 0);
    assert!(custom.validate_default_ref(&1).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn effect_config_defaults_validate_only_absence_and_preserve_decoder_requirements() {
    let source = Arc::new(MemoryConfigSource::new());
    let path = ConfigPath::new([Arc::from("count")]).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let defaults = Arc::clone(&calls);
    let config = EffectConfig::new(
        path.clone(),
        json_i64()
            .refine("positive", |n| *n > 0)
            .into_effect::<MemoryConfigSource, ()>(),
    )
    .with_default(move || {
        let calls = Arc::clone(&defaults);
        Effect::sync(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            7i64
        })
    });
    assert_eq!(success(block_on(config.load().run(Arc::clone(&source)))), 7);
    assert!(source.set(path.clone(), JsonValue::Null).is_none());
    assert!(matches!(
        block_on(config.load().run(Arc::clone(&source))),
        Exit::Fail(_)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(source.set(path, JsonValue::from(9)).is_some());
    assert_eq!(success(block_on(config.load().run(source))), 9);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn late_entity_acquisition_is_released_before_stale_ownership_is_reported() {
    use effect::cluster::*;
    let store = Arc::new(MemoryClusterMessageStore::new(
        MessageStoreCapacity::Bounded(nz(8)),
    ));
    let address = ClusterAddress {
        entity_type: ClusterEntityType::new("reference-resource").unwrap(),
        entity: ClusterEntityId::from_stable(key("entity")),
        shard: ClusterShardId::from_stable(key("shard")),
    };
    let activation = ActivationId::from_stable(key("activation"));
    let owner = match success(run(store.activate(
        address,
        activation,
        UtcMillis::new(1),
        UtcMillis::new(10),
    ))) {
        ActivationOutcome::Activated { lease, .. } => lease.owner,
        _ => panic!("activation must be acquired"),
    };
    let acquired = Arc::new(AtomicUsize::new(0));
    let released = Arc::new(AtomicUsize::new(0));
    let acquire = Arc::clone(&acquired);
    let release = Arc::clone(&released);
    let resource = EntityResource::<String, (), Arc<MemoryClusterMessageStore>>::new(
        move |_| {
            let calls = Arc::clone(&acquire);
            Effect::sync(move || {
                calls.fetch_add(1, Ordering::SeqCst);
                String::from("owned")
            })
        },
        move |value| {
            let calls = Arc::clone(&release);
            Effect::sync_once(move || {
                assert_eq!(value, "owned");
                calls.fetch_add(1, Ordering::SeqCst);
            })
        },
    );
    let result = block_on(
        Scope::scoped(move |scope| {
            resource.open(&scope, owner.clone(), || {
                Effect::succeed(UtcMillis::new(11))
            })
        })
        .run(Arc::new(store)),
    );
    assert!(matches!(
        result,
        Exit::Fail(EntityResourceError::StaleOwnership)
    ));
    assert_eq!(acquired.load(Ordering::SeqCst), 1);
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn object_patterns_apply_all_matches_and_allof_rejects_conflicting_encodings() {
    let pattern = |source| {
        CheckedPattern::new(
            source,
            PatternPolicy {
                compiled_bytes: nz(16384),
                syntax_depth: nz(16),
                steps: nz(4096),
            },
        )
        .unwrap()
    };
    let codec = json_object_constraints(
        ObjectConstraints {
            names: json_string(),
            declared: BTreeMap::new(),
            patterns: vec![
                PatternProperty {
                    pattern: pattern("^x"),
                    codec: json_i64().refine("positive", |n| *n > 0),
                },
                PatternProperty {
                    pattern: pattern("x$"),
                    codec: json_i64().refine("even", |n| *n % 2 == 0),
                },
            ],
            additional: None,
            minimum: 0,
            maximum: 4,
            multiple_patterns: MultiplePatternPolicy::ValidateAll,
        },
        policy(),
    )
    .unwrap();
    assert!(codec.decode_ref(&serde_json::json!({"x":2})).is_ok());
    assert!(codec.decode_ref(&serde_json::json!({"x":3})).is_err());
    assert!(codec.decode_ref(&serde_json::json!({"other":2})).is_err());
    let left = PureCodec::new(
        |_: &JsonValue| Ok(1i64),
        |_: &i64| Ok(serde_json::json!({"x":1})),
    );
    let right = PureCodec::new(
        |_: &JsonValue| Ok(1i64),
        |_: &i64| Ok(serde_json::json!({"x":2})),
    );
    let all = json_all_of(vec![left, right], 0, policy()).unwrap();
    assert!(matches!(
        all.encode_ref(&1).unwrap_err().kind(),
        SchemaIssueKind::MergeConflict
    ));
}

#[test]
fn reactive_generations_and_old_source_cleanup_cannot_reach_a_replacement() {
    use effect::reactivity::*;
    let registry = AtomRegistry::new(PubSubOptions {
        capacity: nz(4),
        overflow: PubSubOverflow::DropOldest,
        replay: ReplayPolicy::None,
    })
    .unwrap();
    let derived = AtomId::from_stable(key("derived"));
    registry
        .register(derived.clone(), AtomKind::Derived)
        .unwrap();
    let first = registry.begin_evaluation(&derived).unwrap();
    assert!(success(run(registry.remove(derived.clone()))));
    registry
        .register(derived.clone(), AtomKind::Derived)
        .unwrap();
    let second = registry.begin_evaluation(&derived).unwrap();
    assert_ne!(first, second);
    assert!(!registry.is_current(&derived, first));
    assert!(registry.is_current(&derived, second));
    struct Owned(String);
    let source = AtomId::from_stable(key("source"));
    let old = AtomRef::new(
        registry.clone(),
        source.clone(),
        Owned("old".into()),
        nz(4),
        PubSubOverflow::DropOldest,
    )
    .unwrap();
    assert!(success(run(registry.remove(source.clone()))));
    let new = AtomRef::new(
        registry.clone(),
        source.clone(),
        Owned("new".into()),
        nz(4),
        PubSubOverflow::DropOldest,
    )
    .unwrap();
    let rejected = run(old.set(Owned("stale".into())));
    assert!(matches!(
        rejected,
        Exit::Fail(AtomRefError::Registry(AtomRegistryError::StaleOwner(_)))
    ));
    let stale_source = old.clone();
    let reader = Atom::<(), AtomRefError, ()>::new(
        registry.clone(),
        AtomId::from_stable(key("old-handle-reader")),
        nz(4),
        PubSubOverflow::DropOldest,
        move |context| context.read(&stale_source).map(|_| ()),
    )
    .unwrap();
    success(run(reader.refresh()));
    let status = success(run(reader.read()));
    match status.as_ref() {
        AsyncResult::Settled {
            outcome: Exit::Fail(error),
            ..
        } => assert!(
            error
                .with(|error| matches!(
                    error,
                    AtomError::Evaluation(AtomRefError::Registry(AtomRegistryError::StaleOwner(_)))
                ))
                .unwrap()
        ),
        _ => panic!("expected stale owner failure in settled status"),
    }
    success(run(reader.dispose(PubSubShutdown::Discard)));
    let calls = Arc::new(AtomicUsize::new(0));
    let capture = Arc::clone(&calls);
    let old_derived = Atom::<(), (), ()>::new(
        registry.clone(),
        AtomId::from_stable(key("old-derived")),
        nz(4),
        PubSubOverflow::DropOldest,
        move |_| {
            capture.fetch_add(1, Ordering::SeqCst);
            Effect::succeed(())
        },
    )
    .unwrap();
    success(run(registry.remove(old_derived.id().clone())));
    registry
        .register(old_derived.id().clone(), AtomKind::Derived)
        .unwrap();
    assert!(matches!(
        run(old_derived.refresh()),
        Exit::Fail(AtomError::Registry(AtomRegistryError::StaleOwner(_)))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let stale_derived = old_derived.clone();
    let reader = Atom::<(), AtomDependencyError<()>, ()>::new(
        registry.clone(),
        AtomId::from_stable(key("old-derived-reader")),
        nz(4),
        PubSubOverflow::DropOldest,
        move |context| context.read_derived(&stale_derived).map(|_| ()),
    )
    .unwrap();
    success(run(reader.refresh()));
    let status = success(run(reader.read()));
    match status.as_ref() {
        AsyncResult::Settled {
            outcome: Exit::Fail(error),
            ..
        } => assert!(
            error
                .with(|error| matches!(
                    error,
                    AtomError::Evaluation(AtomDependencyError::Unavailable)
                ))
                .unwrap()
        ),
        _ => panic!("expected unavailable replaced derived dependency"),
    }
    success(run(reader.dispose(PubSubShutdown::Discard)));
    success(run(old_derived.dispose(PubSubShutdown::Discard)));
    success(run(old.dispose(PubSubShutdown::Discard)));
    assert!(registry.dependencies_of(&source).is_ok());
    assert_eq!(success(run(new.read())).0, "new");
    success(run(new.dispose(PubSubShutdown::Discard)));
    success(run(registry.dispose(PubSubShutdown::Discard)));
}

fn configured_policy() -> SchemaPolicy {
    SchemaPolicy {
        parse: policy(),
        json_depth: nz(16),
        string_length: nz(64),
        integer_digits: nz(20),
        pattern_steps: nz(16384),
    }
}
#[test]
fn configured_patterns_share_one_budget_through_pure_and_native_composition() {
    let pattern = CheckedPattern::new(
        "a+",
        PatternPolicy {
            compiled_bytes: nz(16384),
            syntax_depth: nz(16),
            steps: nz(16384),
        },
    )
    .unwrap();
    let text = json_string_constraints(StringConstraints {
        minimum: 1,
        maximum: 10,
        pattern: Some(pattern),
        uuid_v4: false,
    })
    .unwrap();
    let mut limits = configured_policy();
    limits.pattern_steps = nz(1);
    let pure = json_array(text.clone(), None)
        .into_effect::<(), ()>()
        .configured(limits)
        .unwrap();
    let native = native_array(text.into_effect::<(), ()>(), JsonRepresentation, policy())
        .configured(limits)
        .unwrap();
    for codec in [pure, native] {
        for _ in 0..2 {
            assert!(
                matches!(run(codec.parse("[\"aa\"]")),Exit::Fail(issue) if matches!(issue.kind(),SchemaIssueKind::ResourceDemand{resource:SchemaResource::PatternSteps,..}))
            );
            assert!(
                matches!(run(codec.validate_once(vec!["aa".into()])),Exit::Fail(issue) if issue.path()==[IssueSegment::Index(0)])
            );
        }
    }
}
#[test]
fn configured_native_uses_run_collection_and_error_policies_in_all_directions() {
    struct Pair {
        left: i64,
        right: i64,
    }
    let field = json_i64()
        .refine("positive", |value| *value > 0)
        .into_effect::<(), ()>();
    let codec=schema! {struct Pair {left:field.clone(),right:field} policy:policy(),representation:JsonRepresentation,unknown_fields:UnknownFields::Reject}.unwrap();
    let encoded = match run(codec.encode_once(Pair {
        left: -1,
        right: -2,
    })) {
        Exit::Fail(issue) => issue,
        _ => panic!("encode must accumulate"),
    };
    let validated = match run(codec.validate_once(Pair {
        left: -1,
        right: -2,
    })) {
        Exit::Fail(issue) => issue,
        _ => panic!("validate must accumulate"),
    };
    for issue in [encoded, validated] {
        assert_eq!(issue.children().len(), 2);
        assert!(issue.children().iter().all(|child| child.path().len() == 1));
    }
    let mut limits = configured_policy();
    limits.parse.errors = SchemaErrorMode::First;
    let first = codec.configured(limits).unwrap();
    assert!(
        matches!(run(first.validate_once(Pair{left:-1,right:-2})),Exit::Fail(issue) if issue.children().is_empty())
    );
    limits.parse.limits.collection = nz(1);
    let array = native_array(
        json_i64().into_effect::<(), ()>(),
        JsonRepresentation,
        policy(),
    )
    .configured(limits)
    .unwrap();
    assert!(matches!(
        run(array.validate_once(vec![1, 2])),
        Exit::Fail(_)
    ));
    assert!(matches!(run(array.encode_once(vec![1, 2])), Exit::Fail(_)));
}
#[test]
fn configured_json_preflight_reports_nested_value_and_property_name_paths() {
    let mut limits = configured_policy();
    limits.string_length = nz(2);
    let codec = Codec::new(
        |value: JsonValue| Effect::<_, SchemaIssue>::from_result_once(Ok(value)),
        |value| Effect::<_, SchemaIssue>::from_result_once(Ok(value)),
    )
    .configured(limits)
    .unwrap();
    assert!(
        matches!(run(codec.decode_once(serde_json::json!({"x":["long"]}))),Exit::Fail(issue) if issue.path()==[IssueSegment::Field("x".into()),IssueSegment::Index(0)])
    );
    assert!(
        matches!(run(codec.decode_once(serde_json::json!({"x":{"long":1}}))),Exit::Fail(issue) if issue.path()==[IssueSegment::Field("x".into()),IssueSegment::PropertyName("long".into())])
    );
}
#[test]
fn issue_budget_counts_nested_children_and_stops_before_later_alternatives() {
    let calls = Arc::new(AtomicUsize::new(0));
    let later = Arc::clone(&calls);
    let bad = PureCodec::new(
        |_: &JsonValue| {
            Err::<i64, _>(
                SchemaIssue::new(SchemaIssueKind::Multiple).with_children(vec![
                    SchemaIssue::new(SchemaIssueKind::OutOfRange),
                    SchemaIssue::new(SchemaIssueKind::OutOfRange),
                ]),
            )
        },
        |_: &i64| Ok(JsonValue::Null),
    );
    let later = PureCodec::new(
        move |_: &JsonValue| {
            later.fetch_add(1, Ordering::SeqCst);
            Ok(1)
        },
        |_: &i64| Ok(JsonValue::Null),
    );
    let mut bounded = policy();
    bounded.limits.issues = nz(2);
    let codec = json_any_of(vec![bad, later], bounded).unwrap();
    assert!(
        matches!(codec.decode_ref(&JsonValue::Null),Err(issue) if matches!(issue.kind(),SchemaIssueKind::LimitExceeded{resource:SchemaResource::Issues}))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[test]
fn writer_witness_must_write_one_value_and_keep_the_owned_writer() {
    let zero = PureCodec::new(|_: &JsonValue| Ok(()), |_: &()| Ok(JsonValue::Null))
        .with_writer(|_, _| Ok(()));
    let two = PureCodec::new(|_: &JsonValue| Ok(()), |_: &()| Ok(JsonValue::Null)).with_writer(
        |_, writer| {
            writer.i64(1)?;
            writer.i64(2)
        },
    );
    let bounds = JsonWriterPolicy {
        bytes: nz(64),
        depth: nz(8),
    };
    assert!(zero.write(&(), bounds).is_err());
    let mut writer = JsonWriter::new(bounds);
    writer.begin_array().unwrap();
    assert!(two.write_into(&(), &mut writer).is_err());
    assert!(writer.finish().is_err());
    let replaced = Codec::new(
        |_: JsonValue| Effect::<_, SchemaIssue>::succeed(()),
        |_: ()| Effect::succeed(JsonValue::Null),
    )
    .with_writer(move |_, _| {
        let mut replacement = JsonWriter::new(bounds);
        replacement.i64(1).unwrap();
        Effect::from_result_once(Ok(replacement))
    });
    assert!(matches!(
        run(replaced.write_once((), bounds)),
        Exit::Fail(_)
    ));
}
#[test]
fn constrained_and_tagged_defaults_validate_and_write_without_encoder_replay() {
    let options = json_option(json_i64(), "tag", "value", "some", "none", policy()).unwrap();
    assert!(options.validate_default_ref(&Some(3)).is_ok());
    let unique = json_array_constraints(
        json_i64(),
        ArrayConstraints {
            minimum: 1,
            maximum: 4,
            unique: true,
        },
        policy(),
    )
    .unwrap();
    assert!(unique.validate_default_ref(&vec![1, 2]).is_ok());
    assert!(unique.validate_default_ref(&vec![1, 1]).is_err());
    let calls = Arc::new(AtomicUsize::new(0));
    let encodes = Arc::clone(&calls);
    let leaf = PureCodec::new(
        |value: &JsonValue| Ok(value.as_i64().unwrap()),
        move |value: &i64| {
            encodes.fetch_add(1, Ordering::SeqCst);
            Ok((*value).into())
        },
    )
    .with_type_validator(|_| Ok(()))
    .with_writer(|value, writer| writer.i64(*value));
    let prefix = json_prefix_rest(
        vec![PrefixElement {
            codec: leaf,
            required: true,
        }],
        None::<PureCodec<i64, JsonValue>>,
        policy(),
    )
    .unwrap();
    let value = PrefixValue {
        prefix: vec![Some(1)],
        rest: vec![],
    };
    assert!(prefix.validate_default_ref(&value).is_ok());
    assert!(
        prefix
            .write(
                &value,
                JsonWriterPolicy {
                    bytes: nz(64),
                    depth: nz(8)
                }
            )
            .is_ok()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[test]
fn derived_first_build_tracks_source_changes_before_dependency_installation() {
    use effect::reactivity::*;
    let registry = AtomRegistry::new(PubSubOptions {
        capacity: nz(8),
        overflow: PubSubOverflow::DropOldest,
        replay: ReplayPolicy::None,
    })
    .unwrap();
    let source = AtomRef::new(
        registry.clone(),
        AtomId::from_stable(key("raced-source")),
        1i64,
        nz(8),
        PubSubOverflow::DropOldest,
    )
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let source_for_eval = source.clone();
    let atom = Atom::<i64, AtomRefError, ()>::new(
        registry.clone(),
        AtomId::from_stable(key("raced-derived")),
        nz(8),
        PubSubOverflow::DropOldest,
        move |context| {
            let source = source_for_eval.clone();
            let calls = Arc::clone(&calls);
            context.read(&source).flat_map(move |value| {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    source.set(2).map(move |_| *value)
                } else {
                    Effect::succeed(*value)
                }
            })
        },
    )
    .unwrap();
    success(run(atom.refresh()));
    assert_eq!(
        registry.dependencies_of(atom.id()).unwrap(),
        [source.id().clone()].into_iter().collect()
    );
    let status = success(run(atom.read()));
    assert!(matches!(
        status.as_ref(),
        AsyncResult::Settled {
            outcome: Exit::Interrupted,
            ..
        }
    ));
    success(run(atom.refresh()));
    let status = success(run(atom.read()));
    assert!(
        matches!(status.as_ref(),AsyncResult::Settled{outcome:Exit::Success(value),..} if value.with(|value|*value).unwrap()==2)
    );
}
#[test]
fn suspended_effectful_recursive_runs_keep_fresh_contexts_without_cloning_payloads() {
    struct Tree(Vec<Tree>);
    let codec = Codec::<Tree, JsonValue, (), ()>::recursive(
        "owned-tree",
        |reference| SchemaShape::Sequence {
            element: SchemaAst::new(SchemaShape::Reference(reference)),
            maximum: None,
        },
        |reference| {
            Ok(
                native_array(reference, JsonRepresentation, policy()).transform_effectful(
                    |children| {
                        Effect::from_future_once(move |_| async move {
                            let mut pending = true;
                            futures_util::future::poll_fn(move |cx| {
                                if pending {
                                    pending = false;
                                    cx.waker().wake_by_ref();
                                    std::task::Poll::Pending
                                } else {
                                    std::task::Poll::Ready(())
                                }
                            })
                            .await;
                            Tree(children)
                        })
                    },
                    |tree| Effect::from_future_once(move |_| async move { tree.0 }),
                ),
            )
        },
    )
    .unwrap();
    let mut limits = configured_policy();
    limits.parse.limits.nodes = nz(6);
    limits.parse.limits.depth = nz(4);
    let configured = codec.configured(limits).unwrap();
    let (left, right) = block_on(futures_util::future::join(
        configured.parse("[[],[]]").run(Arc::new(())),
        configured.parse("[[],[]]").run(Arc::new(())),
    ));
    assert_eq!(success(left).0.len(), 2);
    assert_eq!(success(right).0.len(), 2);
    assert!(
        matches!(run(configured.parse("[[[[[]]]]]")),Exit::Fail(issue) if !issue.path().is_empty())
    );
    assert_eq!(
        success(run(configured.encode_once(Tree(vec![Tree(vec![])])))),
        serde_json::json!([[]])
    );
}
#[test]
fn configured_native_defaults_preserve_pattern_and_schema_traversal_budgets() {
    struct DefaultText {
        text: String,
    }
    let pattern = CheckedPattern::new(
        "a+",
        PatternPolicy {
            compiled_bytes: nz(16384),
            syntax_depth: nz(16),
            steps: nz(16384),
        },
    )
    .unwrap();
    let field = json_string_constraints(StringConstraints {
        minimum: 1,
        maximum: 10,
        pattern: Some(pattern),
        uuid_v4: false,
    })
    .unwrap()
    .into_effect::<(), ()>();
    let codec=schema!{struct DefaultText{#[schema(default_with=||Effect::sync(||String::from("aa")))]text:field}policy:policy(),representation:JsonRepresentation,unknown_fields:UnknownFields::Reject}.unwrap();
    let mut bounds = configured_policy();
    bounds.pattern_steps = nz(1);
    let configured = codec.configured(bounds).unwrap();
    assert!(
        matches!(run(configured.parse("{}")),Exit::Fail(issue) if matches!(issue.kind(),SchemaIssueKind::ResourceDemand{resource:SchemaResource::PatternSteps,..})&&issue.path()==[IssueSegment::Field("text".into())])
    );
}

#[test]
fn suspended_direct_writer_keeps_owned_nonclone_payload_and_checked_output() {
    struct Owned(i64);
    let codec = Codec::new(
        |value: JsonValue| {
            Effect::<_, SchemaIssue>::from_result_once(
                value
                    .as_i64()
                    .map(Owned)
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange)),
            )
        },
        |value: Owned| Effect::<_, SchemaIssue>::succeed(value.0.into()),
    )
    .with_writer(|value, mut writer| {
        Effect::from_future_once(move |_| async move {
            let mut suspended = false;
            futures_util::future::poll_fn(move |cx| {
                if suspended {
                    std::task::Poll::Ready(())
                } else {
                    suspended = true;
                    cx.waker().wake_by_ref();
                    std::task::Poll::Pending
                }
            })
            .await;
            writer.i64(value.0).map(|()| writer)
        })
        .flat_map(Effect::from_result_once)
    });
    assert_eq!(
        success(run(codec.write_once(
            Owned(42),
            JsonWriterPolicy {
                bytes: nz(8),
                depth: nz(2)
            }
        ))),
        b"42"
    );
}
