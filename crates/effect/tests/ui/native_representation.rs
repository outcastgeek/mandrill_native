use mandrill_effect::*;
struct Record{value:i64}
fn main(){let _=schema!{struct Record{value:json_i64().into_effect::<(),()>()}policy:panic!("unused"),representation:(),unknown_fields:UnknownFields::Reject};}
