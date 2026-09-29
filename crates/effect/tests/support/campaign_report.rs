pub fn completed(name: &str, counts: serde_json::Value) {
    let Some(path) = std::env::var_os("EFFECT_CAMPAIGN_REPORT") else {
        return;
    };
    let report = serde_json::json!({ "campaign": name, "completed": counts });
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&report).expect("campaign report JSON"),
    )
    .expect("campaign report is writable");
}
