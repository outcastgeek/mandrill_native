use mandrill_effect::ResourceValue;

fn escape(value: &ResourceValue<String>) -> &str {
    value.with(|text| text.as_str()).unwrap_or_default()
}

fn main() { let _ = escape; }
