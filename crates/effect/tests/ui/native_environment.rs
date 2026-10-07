use mandrill_effect::*;
struct Record{first:i64,second:i64}
struct First;struct Second;
fn main(){let _=schema!{struct Record{first:json_i64().into_effect::<First,()>(),second:json_i64().into_effect::<Second,()>()}policy:panic!("unused"),representation:JsonRepresentation,unknown_fields:UnknownFields::Reject};}
