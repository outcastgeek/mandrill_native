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
    let _: Result<Record, _> = codec.decode_ref(&JsonValue::Null);
}
