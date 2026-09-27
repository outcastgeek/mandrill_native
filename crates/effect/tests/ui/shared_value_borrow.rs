use mandrill_effect::SharedValue;
fn main() { let value=SharedValue::new(String::new()); let _=value.with(|value|value.as_str()); }
