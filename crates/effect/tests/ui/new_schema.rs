#![deny(semicolon_in_expressions_from_macros)]
use mandrill_effect::{
    DuplicateFields, JsonValue, ParseLimits, ParsePolicy, SchemaErrorMode, UnknownFields, json_i64,
    schema,
};
use std::num::NonZeroUsize;

struct Record {
    count: i64,
}

fn main() {
    let limit = NonZeroUsize::new(64).unwrap();
    let policy = ParsePolicy {
        limits: ParseLimits {
            bytes: limit,
            depth: limit,
            nodes: limit,
            collection: limit,
            issues: limit,
        },
        errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    };
    let codec = schema! { struct Record { count: json_i64() } policy: policy }.unwrap();
    let native=schema!{struct Record{#[schema(default_with = ||mandrill_effect::Effect::succeed(1i64))]count:json_i64().into_effect::<(),()>()}policy:policy,representation:mandrill_effect::JsonRepresentation,unknown_fields:UnknownFields::Reject}.unwrap();
    let _ = native.decode_once(JsonValue::Null);
    let _: Result<Record, _> = codec.decode_ref(&JsonValue::Null);
    published_example::exercise(policy);
}

mod published_example {
    use mandrill_effect::{
        Codec, JsonRepresentation, JsonValue, ParsePolicy, SchemaIssue, UnknownFields, json_i64,
        schema,
    };
    struct Count {
        count: i64,
    }
    fn count_schema(policy: ParsePolicy) -> Result<Codec<Count, JsonValue, (), ()>, SchemaIssue> {
        schema! {
            struct Count {
                #[schema(default_with = || mandrill_effect::Effect::succeed(1i64))]
                count: json_i64().into_effect::<(), ()>()
            }
            policy: policy,
            representation: JsonRepresentation,
            unknown_fields: UnknownFields::Reject
        }
    }
    pub fn exercise(policy: ParsePolicy) {
        let codec = count_schema(policy).unwrap();
        let _ = codec.encode_once(Count { count: 1 });
        let _ = Count { count: 1 }.count;
    }
}
