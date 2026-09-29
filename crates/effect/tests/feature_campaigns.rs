use effect::{
    DuplicateFields, Exit, JsonValue, ParseLimits, ParsePolicy, PrimitiveShape, PureCodec,
    RepresentationError, RepresentationLimits, RepresentationPrecision, SchemaAst, SchemaErrorMode,
    SchemaIssue, SchemaIssueKind, SchemaShape, UnknownFields, export_json_schema, json_bool,
    json_i64, json_option, json_string,
};
use futures_executor::block_on;
#[path = "support/campaign_report.rs"]
mod campaign_report;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

#[derive(Debug, PartialEq)]
enum List {
    Nil,
    Cons(i64, Box<List>),
}

fn invalid_list() -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::InvalidValue {
        code: Arc::from("campaign-list"),
    })
}

fn list_codec() -> PureCodec<List, JsonValue> {
    PureCodec::recursive(
        "campaign-list",
        |_| SchemaShape::Opaque,
        |self_codec| {
            let decode_tail = self_codec.clone();
            Ok(PureCodec::new(
                move |encoded| match encoded {
                    JsonValue::Array(items) if items.is_empty() => Ok(List::Nil),
                    JsonValue::Array(items) if items.len() == 2 => {
                        let head = json_i64().decode_ref(&items[0])?;
                        let tail = decode_tail.decode_ref(&items[1])?;
                        Ok(List::Cons(head, Box::new(tail)))
                    }
                    _ => Err(invalid_list()),
                },
                move |value| match value {
                    List::Nil => Ok(JsonValue::Array(Vec::new())),
                    List::Cons(head, tail) => Ok(JsonValue::Array(vec![
                        json_i64().encode_ref(head)?,
                        self_codec.encode_ref(tail)?,
                    ])),
                },
            ))
        },
    )
    .expect("recursive campaign codec")
}

// Xorshift64 is a specified, reproducible input generator. The seed and
// workload are read from the campaign configuration, not production policy.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn config_number(root: &JsonValue, path: &[&str]) -> usize {
    path.iter()
        .fold(root, |value, key| {
            value
                .get(*key)
                .unwrap_or_else(|| panic!("missing campaign configuration key {key}"))
        })
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .expect("positive campaign configuration integer")
}

struct DecodeEnvironment;
struct EncodeEnvironment;
struct AlternateDecodeEnvironment;

fn run_first_use_contention(config: &JsonValue, policy: ParsePolicy) {
    let callers = config_number(config, &["campaigns", "schema_first_use", "callers"]);
    let rounds = config_number(config, &["campaigns", "schema_first_use", "rounds"]);
    assert!(callers >= 2 && rounds >= 2);
    let values = list_codec();
    let schemas: Vec<_> = (0..rounds)
        .map(|_| {
            let ast = SchemaAst::recursive("first-use-list", |reference| {
                SchemaShape::Union(vec![
                    SchemaAst::new(SchemaShape::Tuple(Vec::new())),
                    SchemaAst::new(SchemaShape::Tuple(vec![
                        SchemaAst::primitive(PrimitiveShape::Integer),
                        SchemaAst::new(SchemaShape::Reference(reference)),
                    ])),
                ])
            })
            .expect("recursive first-use schema");
            let decode = values.clone();
            let encode = values.clone();
            let codec = PureCodec::with_ast(
                ast,
                move |input| decode.decode_ref(input),
                move |value| encode.encode_ref(value),
            );
            (
                codec
                    .into_effect::<DecodeEnvironment, EncodeEnvironment>()
                    .as_effect_schema(),
                codec
                    .into_effect::<AlternateDecodeEnvironment, EncodeEnvironment>()
                    .as_effect_schema(),
            )
        })
        .collect();
    let start = Arc::new(Barrier::new(callers));
    std::thread::scope(|scope| {
        for caller in 0..callers {
            let start = Arc::clone(&start);
            let schemas = &schemas;
            scope.spawn(move || {
                let first_environment = Arc::new(DecodeEnvironment);
                let alternate_environment = Arc::new(AlternateDecodeEnvironment);
                for (first, alternate) in schemas {
                    start.wait();
                    let result = if caller % 2 == 0 {
                        block_on(
                            first
                                .parse("[]", policy)
                                .run(Arc::clone(&first_environment)),
                        )
                    } else {
                        block_on(
                            alternate
                                .parse("[]", policy)
                                .run(Arc::clone(&alternate_environment)),
                        )
                    };
                    assert!(matches!(result, Exit::Success(List::Nil)));
                }
            });
        }
    });
    let export_limits = RepresentationLimits {
        depth: policy.limits.depth,
        nodes: policy.limits.nodes,
        definitions: NonZeroUsize::MIN,
    };
    let recursive_document = export_json_schema(schemas[0].0.ast(), export_limits)
        .expect("recursive schema exports with an explicit definition budget");
    assert_eq!(
        recursive_document.precision(),
        RepresentationPrecision::Approximate
    );
    assert!(
        recursive_document.value()["$defs"]
            .get("first-use-list")
            .is_some()
    );
    assert!(matches!(
        export_json_schema(
            schemas[0].0.ast(),
            RepresentationLimits {
                nodes: NonZeroUsize::MIN,
                ..export_limits
            }
        ),
        Err(RepresentationError::NodeLimit)
    ));
    let boolean_document = export_json_schema(json_bool().ast(), export_limits)
        .expect("primitive boolean schema exports exactly");
    assert_eq!(boolean_document.precision(), RepresentationPrecision::Exact);
    assert_eq!(boolean_document.value()["type"], JsonValue::from("boolean"));
    eprintln!(
        "schema first-use contention completed {rounds} rounds with {callers} callers across two environment types"
    );
}

#[test]
fn schema_generation_configured_seeds_and_boundary_corpus() {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/feature_campaigns.json")
        });
    let config: JsonValue = std::fs::read_to_string(configured)
        .expect("campaign configuration is readable")
        .parse()
        .expect("valid campaign configuration");
    let seed = config_number(&config, &["seed"]);
    let samples = config_number(&config, &["campaigns", "schema", "seeds"]);
    let bytes = config_number(&config, &["budgets", "bytes"]);
    let depth = config_number(&config, &["budgets", "depth"]);
    let width = config_number(&config, &["budgets", "width"]);
    assert!(seed > 0 && samples >= 1000 && bytes >= 2 && depth > 1 && width > 0);
    let limits = ParseLimits {
        bytes: NonZeroUsize::new(bytes).expect("byte budget"),
        depth: NonZeroUsize::new(depth).expect("depth budget"),
        nodes: NonZeroUsize::new(config_number(&config, &["budgets", "steps"]))
            .expect("node budget"),
        collection: NonZeroUsize::new(width).expect("collection budget"),
        issues: NonZeroUsize::new(width).expect("issue budget"),
    };
    let policy = ParsePolicy {
        limits,
        errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    };
    let numbers = json_i64();
    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_witness = Arc::clone(&callback_count);
    let checked = numbers.clone().refine("counted-campaign-check", move |_| {
        callback_witness.fetch_add(1, Ordering::SeqCst);
        true
    });
    let recursive = list_codec();
    let shared = recursive.clone();
    let optional = json_option(numbers.clone(), "kind", "value", "some", "none", policy)
        .expect("option campaign codec");
    let directional = numbers
        .clone()
        .into_effect::<DecodeEnvironment, EncodeEnvironment>();
    let mut generator = seed as u64;
    for _ in 0..samples {
        let value = next(&mut generator) as i64;
        let encoded = numbers.encode_ref(&value).expect("encode integer");
        assert_eq!(numbers.decode_ref(&encoded).expect("decode integer"), value);
        let checked_value = checked.encode_ref(&value).expect("encode checked integer");
        assert_eq!(
            checked
                .decode_ref(&checked_value)
                .expect("decode checked integer"),
            value
        );
        assert!(numbers.decode_ref(&JsonValue::Bool(true)).is_err());
        let option = if next(&mut generator) & 1 == 0 {
            Some(value)
        } else {
            None
        };
        let encoded_option = optional.encode_ref(&option).expect("encode option");
        assert_eq!(
            optional.decode_ref(&encoded_option).expect("decode option"),
            option
        );
        let length = next(&mut generator) as usize % width.min(depth);
        let mut list = List::Nil;
        for _ in 0..length {
            list = List::Cons(value, Box::new(list));
        }
        let encoded_list = recursive.encode_ref(&list).expect("encode recursive list");
        assert_eq!(
            shared
                .decode_ref(&encoded_list)
                .expect("decode shared list"),
            list
        );
        assert_eq!(
            recursive
                .parse_str(&encoded_list.to_string(), policy)
                .expect("parse recursive list"),
            list
        );
        assert!(matches!(block_on(directional.decode_once(encoded.clone())
            .run(Arc::new(DecodeEnvironment))), Exit::Success(found) if found == value));
        assert!(matches!(block_on(directional.encode_once(value)
            .run(Arc::new(EncodeEnvironment))), Exit::Success(found) if found == encoded));
    }
    let exact = format!("\"{}\"", "a".repeat(bytes - 2));
    assert_eq!(callback_count.load(Ordering::SeqCst), samples * 2);
    assert!(json_string().parse_str(&exact, policy).is_ok());
    let oversized = format!("\"{}\"", "a".repeat(bytes - 1));
    assert!(json_string().parse_str(&oversized, policy).is_err());
    run_first_use_contention(&config, policy);
    campaign_report::completed(
        "schema",
        serde_json::json!({
            "seeds": samples,
            "callers": config_number(&config, &["campaigns", "schema_first_use", "callers"]),
            "rounds": config_number(&config, &["campaigns", "schema_first_use", "rounds"]),
        }),
    );
    eprintln!("schema campaign completed {samples} configured seeds and byte-limit boundaries");
}
