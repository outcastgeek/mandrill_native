use serde_json::json;
pub fn completed(name: &str, counts: serde_json::Value) -> Result<(), String> {
    let Some(path) = std::env::var_os("EFFECT_CAMPAIGN_REPORT") else {
        return Ok(());
    };
    let report = json!({ "campaign": name, "completed": counts });
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&report)
            .map_err(|error| format!("campaign report JSON: {error}"))?,
    )
    .map_err(|error| format!("campaign report is writable: {error}"))?;
    Ok(())
}
