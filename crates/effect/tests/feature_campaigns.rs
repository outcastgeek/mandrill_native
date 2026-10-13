use effect::{
    DuplicateFields, Exit, JsonValue, ParseLimits, ParsePolicy, PrimitiveShape, PureCodec,
    RepresentationError, RepresentationLimits, RepresentationPrecision, SchemaAst, SchemaErrorMode,
    SchemaIssue, SchemaIssueKind, SchemaShape, UnknownFields, export_json_schema, json_bool,
    json_i64, json_option, json_string,
};
use effect::{codec, effect_schema_parse};
use futures_executor::block_on;
use serde_json::json;
#[path = "support/campaign_report.rs"]
mod campaign_report;
use std::io::Write;
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

fn list_codec() -> Result<PureCodec<List, JsonValue>, SchemaIssue> {
    codec! { recursive_pure; name: "campaign-list", shape: |_| SchemaShape::Opaque, build: |self_codec| {
        let decode_tail = self_codec.clone();
        Ok(codec! { pure; decode: move |encoded| match encoded {
                JsonValue::Array(items) if items.is_empty() => Ok(List::Nil),
                JsonValue::Array(items) if items.len() == 2 => {
                    let [head, tail] = items.as_slice() else {
                        return Err(invalid_list());
                    };
                    let head = json_i64().decode_ref(head)?;
                    let tail = decode_tail.decode_ref(tail)?;
                    Ok(List::Cons(head, Box::new(tail)))
                }
                JsonValue::Array(_)
                | JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::String(_)
                | JsonValue::Object(_) => Err(invalid_list()),
            }, encode: move |value| match value {
                List::Nil => Ok(JsonValue::Array(Vec::new())),
                List::Cons(head, tail) => Ok(JsonValue::Array(vec![
                    json_i64().encode_ref(head)?,
                    self_codec.encode_ref(tail)?,
                ])),
            }, })
    }, }
}

// Xorshift64 is a specified, reproducible input generator. The seed and
// workload are read from the campaign configuration, not production policy.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn config_number(root: &JsonValue, path: &[&str]) -> Result<usize, String> {
    path.iter()
        .try_fold(root, |value, key| {
            value
                .get(*key)
                .ok_or_else(|| format!("missing campaign configuration key {key}"))
        })?
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| format!("campaign integer: {path:?}"))
}

struct DecodeEnvironment;
struct EncodeEnvironment;
struct AlternateDecodeEnvironment;

fn run_first_use_contention(
    config: &JsonValue,
    policy: ParsePolicy,
) -> Result<(), Box<dyn std::error::Error>> {
    let callers = config_number(config, &["campaigns", "schema_first_use", "callers"])?;
    let rounds = config_number(config, &["campaigns", "schema_first_use", "rounds"])?;
    assert!(callers >= 2 && rounds >= 2);
    let values = list_codec()?;
    let schemas: Vec<_> = (0..rounds)
        .map(|_| -> Result<_, SchemaIssue> {
            let ast = SchemaAst::recursive("first-use-list", |reference| {
                SchemaShape::Union(vec![
                    SchemaAst::new(SchemaShape::Tuple(Vec::new())),
                    SchemaAst::new(SchemaShape::Tuple(vec![
                        SchemaAst::primitive(PrimitiveShape::Integer),
                        SchemaAst::new(SchemaShape::Reference(reference)),
                    ])),
                ])
            })?;
            let decode = values.clone();
            let encode = values.clone();
            let codec = codec! { pure; ast: ast, decode: move |input| decode.decode_ref(input), encode: move |value| encode.encode_ref(value), };
            Ok((
                codec
                    .into_effect::<DecodeEnvironment, EncodeEnvironment>()
                    .as_effect_schema(),
                codec
                    .into_effect::<AlternateDecodeEnvironment, EncodeEnvironment>()
                    .as_effect_schema(),
            ))
        })
        .collect::<Result<_, _>>()?;
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
                            effect_schema_parse! { codec: first, text: "[]", policy: policy, }
                                .run(Arc::clone(&first_environment)),
                        )
                    } else {
                        block_on(
                            effect_schema_parse! { codec: alternate, text: "[]", policy: policy, }
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
    let first = schemas.first().ok_or("missing first-use schema")?;
    let recursive_document = export_json_schema(first.0.ast(), export_limits)
        .map_err(|error| format!("recursive schema export: {error:?}"))?;
    assert_eq!(
        recursive_document.precision(),
        RepresentationPrecision::Approximate
    );
    assert!(
        recursive_document
            .value()
            .get("$defs")
            .ok_or("missing definitions")?
            .get("first-use-list")
            .is_some()
    );
    assert!(matches!(
        export_json_schema(
            first.0.ast(),
            RepresentationLimits {
                nodes: NonZeroUsize::MIN,
                ..export_limits
            }
        ),
        Err(RepresentationError::NodeLimit)
    ));
    let boolean_document = export_json_schema(json_bool().ast(), export_limits)
        .map_err(|error| format!("boolean schema export: {error:?}"))?;
    assert_eq!(boolean_document.precision(), RepresentationPrecision::Exact);
    assert_eq!(
        boolean_document.value().get("type"),
        Some(&JsonValue::from("boolean"))
    );
    writeln!(
        std::io::stderr().lock(),
        "schema first-use contention completed {rounds} rounds with {callers} callers across two environment types"
    )?;
    Ok(())
}

#[test]
fn schema_generation_configured_seeds_and_boundary_corpus() -> Result<(), Box<dyn std::error::Error>>
{
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/feature_campaigns.json")
        });
    let config: JsonValue = std::fs::read_to_string(configured)?.parse()?;
    let seed = config_number(&config, &["seed"])?;
    let samples = config_number(&config, &["campaigns", "schema", "seeds"])?;
    let bytes = config_number(&config, &["budgets", "bytes"])?;
    let depth = config_number(&config, &["budgets", "depth"])?;
    let width = config_number(&config, &["budgets", "width"])?;
    assert!(seed > 0 && samples >= 1000 && bytes >= 2 && depth > 1 && width > 0);
    let limits = ParseLimits {
        bytes: NonZeroUsize::new(bytes).ok_or("zero byte budget")?,
        depth: NonZeroUsize::new(depth).ok_or("zero depth budget")?,
        nodes: NonZeroUsize::new(config_number(&config, &["budgets", "steps"])?)
            .ok_or("zero node budget")?,
        collection: NonZeroUsize::new(width).ok_or("zero collection budget")?,
        issues: NonZeroUsize::new(width).ok_or("zero issue budget")?,
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
    let recursive = list_codec()?;
    let shared = recursive.clone();
    let optional = json_option(numbers.clone(), "kind", "value", "some", "none", policy)?;
    let directional = numbers
        .clone()
        .into_effect::<DecodeEnvironment, EncodeEnvironment>();
    let mut generator = seed as u64;
    for _ in 0..samples {
        let value = next(&mut generator) as i64;
        let encoded = numbers.encode_ref(&value)?;
        assert_eq!(numbers.decode_ref(&encoded)?, value);
        let checked_value = checked.encode_ref(&value)?;
        assert_eq!(checked.decode_ref(&checked_value)?, value);
        assert!(numbers.decode_ref(&JsonValue::Bool(true)).is_err());
        let option = if next(&mut generator) & 1 == 0 {
            Some(value)
        } else {
            None
        };
        let encoded_option = optional.encode_ref(&option)?;
        assert_eq!(optional.decode_ref(&encoded_option)?, option);
        let length = next(&mut generator) as usize % width.min(depth);
        let mut list = List::Nil;
        for _ in 0..length {
            list = List::Cons(value, Box::new(list));
        }
        let encoded_list = recursive.encode_ref(&list)?;
        assert_eq!(shared.decode_ref(&encoded_list)?, list);
        assert_eq!(
            recursive.parse_str(&encoded_list.to_string(), policy)?,
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
    run_first_use_contention(&config, policy)?;
    campaign_report::completed(
        "schema",
        json!({
            "seeds": samples,
            "callers": config_number(&config, &["campaigns", "schema_first_use", "callers"])?,
            "rounds": config_number(&config, &["campaigns", "schema_first_use", "rounds"])?,
        }),
    )?;
    writeln!(
        std::io::stderr().lock(),
        "schema campaign completed {samples} configured seeds and byte-limit boundaries"
    )?;
    Ok(())
}
