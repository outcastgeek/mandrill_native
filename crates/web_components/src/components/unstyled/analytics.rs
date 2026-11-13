#![allow(non_snake_case)]

use crate::components::ComponentInfo;
use maud::{Markup, html};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "analytics",
        name: "GoogleAnalyticsPartytownHead",
    },
    ComponentInfo {
        family: "unstyled",
        module: "analytics",
        name: "GoogleAnalyticsPartytown",
    },
];

const DEFAULT_DATA_LAYER_NAME: &str = "dataLayer";
const DEFAULT_GTAG_FUNCTION_NAME: &str = "gtag";
const DEFAULT_GTAG_SCRIPT_BASE: &str = "https://www.googletagmanager.com/gtag/js";
const DEFAULT_WORKER_SCRIPT_TYPE: &str = "text/partytown";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GoogleAnalyticsPartytownProps {
    pub gtag_id: String,
    pub partytown_url: String,
    pub gtag_script_base_url: String,
    pub data_layer_name: String,
    pub gtag_function_name: String,
    pub worker_script_type: String,
    pub forward: Vec<String>,
    pub partytown_config: Map<String, Value>,
    pub gtag_config_params: Map<String, Value>,
    pub disable_rendering: bool,
    pub skip_partytown_preload: bool,
    pub skip_partytown_script: bool,
    pub skip_partytown_config: bool,
    pub skip_gtag_script: bool,
    pub skip_gtag_init: bool,
    pub partytown_preload_attributes: BTreeMap<String, String>,
    pub partytown_script_attributes: BTreeMap<String, String>,
    pub partytown_config_attributes: BTreeMap<String, String>,
    pub gtag_script_attributes: BTreeMap<String, String>,
    pub gtag_init_attributes: BTreeMap<String, String>,
}

impl GoogleAnalyticsPartytownProps {
    #[must_use]
    pub fn new(gtag_id: impl Into<String>, partytown_url: impl Into<String>) -> Self {
        Self {
            gtag_id: gtag_id.into(),
            partytown_url: partytown_url.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn should_render(&self) -> bool {
        !self.disable_rendering
            && !self.gtag_id.trim().is_empty()
            && !self.partytown_url.trim().is_empty()
    }

    #[must_use]
    pub fn resolved_partytown_url(&self) -> String {
        self.partytown_url.trim().to_string()
    }

    #[must_use]
    pub fn resolved_gtag_script_url(&self) -> String {
        let base = if self.gtag_script_base_url.trim().is_empty() {
            DEFAULT_GTAG_SCRIPT_BASE
        } else {
            self.gtag_script_base_url.trim()
        };
        let sep = if base.contains('?') { '&' } else { '?' };
        let mut url = format!(
            "{base}{sep}id={}",
            encode_query(self.trimmed_gtag_id().as_str())
        );
        let data_layer = self.resolved_data_layer_name();
        if data_layer != DEFAULT_DATA_LAYER_NAME {
            url.push_str("&l=");
            url.push_str(&encode_query(&data_layer));
        }
        url
    }

    #[must_use]
    pub fn resolved_partytown_config(&self) -> Value {
        let mut config = self.partytown_config.clone();
        config.insert("forward".to_string(), json!(self.resolved_forward()));
        Value::Object(config)
    }

    #[must_use]
    pub fn resolved_forward(&self) -> Vec<String> {
        let forward: Vec<String> = self
            .forward
            .iter()
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect();
        if forward.is_empty() {
            vec![format!("{}.push", self.resolved_data_layer_name())]
        } else {
            forward
        }
    }

    #[must_use]
    pub fn resolved_worker_script_type(&self) -> String {
        if self.worker_script_type.trim().is_empty() {
            DEFAULT_WORKER_SCRIPT_TYPE.to_string()
        } else {
            self.worker_script_type.trim().to_string()
        }
    }

    #[must_use]
    pub fn resolved_data_layer_name(&self) -> String {
        let value = self.data_layer_name.trim();
        if is_js_identifier(value) {
            value.to_string()
        } else {
            DEFAULT_DATA_LAYER_NAME.to_string()
        }
    }

    #[must_use]
    pub fn resolved_gtag_function_name(&self) -> String {
        let value = self.gtag_function_name.trim();
        if is_js_identifier(value) {
            value.to_string()
        } else {
            DEFAULT_GTAG_FUNCTION_NAME.to_string()
        }
    }

    #[must_use]
    pub fn trimmed_gtag_id(&self) -> String {
        self.gtag_id.trim().to_string()
    }

    #[must_use]
    pub fn resolved_gtag_config_params(&self) -> Value {
        Value::Object(self.gtag_config_params.clone())
    }
}

#[must_use]
pub fn DefaultGoogleAnalyticsPartytownProps(
    gtag_id: impl Into<String>,
    partytown_url: impl Into<String>,
) -> GoogleAnalyticsPartytownProps {
    GoogleAnalyticsPartytownProps::new(gtag_id, partytown_url)
}

#[must_use]
pub fn is_js_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

#[must_use]
pub fn encode_query(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

#[must_use]
pub fn json_for_script(value: &Value) -> String {
    value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

#[must_use]
fn json_string_for_script(value: &str) -> String {
    json_for_script(&json!(value))
}

#[must_use]
fn attr<'a>(attrs: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    crate::components::shared::attr_value(attrs, name)
}

#[must_use]
pub fn GoogleAnalyticsPartytownHead(props: &GoogleAnalyticsPartytownProps) -> Markup {
    if !props.should_render() {
        return html! {};
    }

    let partytown_url = props.resolved_partytown_url();
    let gtag_url = props.resolved_gtag_script_url();
    let worker_type = props.resolved_worker_script_type();
    let data_layer = json_string_for_script(&props.resolved_data_layer_name());
    let gtag_fn = json_string_for_script(&props.resolved_gtag_function_name());
    let partytown_config = json_for_script(&props.resolved_partytown_config());
    let gtag_id = json_string_for_script(&props.trimmed_gtag_id());
    let gtag_config = json_for_script(&props.resolved_gtag_config_params());
    let partytown_config_script = format!(
        "window[{data_layer}] = window[{data_layer}] || [];\nwindow.partytown = Object.assign({{}}, window.partytown || {{}}, {partytown_config});"
    );
    let gtag_init_script = format!(
        "window[{data_layer}] = window[{data_layer}] || [];\nwindow[{gtag_fn}] = window[{gtag_fn}] || function(){{window[{data_layer}].push(arguments);}};\nwindow[{gtag_fn}]('js', new Date());\nwindow[{gtag_fn}]('config', {gtag_id}, {gtag_config});"
    );

    html! {
        @if !props.skip_partytown_preload {
            link
                rel="preload"
                href=(partytown_url)
                as="script"
                id=[attr(&props.partytown_preload_attributes, "id")]
                data-testid=[attr(&props.partytown_preload_attributes, "data-testid")]
            {}
        }
        @if !props.skip_partytown_script {
            script
                src=(props.resolved_partytown_url())
                defer
                id=[attr(&props.partytown_script_attributes, "id")]
                data-testid=[attr(&props.partytown_script_attributes, "data-testid")]
            {}
        }
        @if !props.skip_partytown_config {
            script
                defer
                id=[attr(&props.partytown_config_attributes, "id")]
                data-testid=[attr(&props.partytown_config_attributes, "data-testid")]
            {
                (partytown_config_script)
            }
        }
        @if !props.skip_gtag_script {
            script
                type=(worker_type.as_str())
                src=(gtag_url)
                id=[attr(&props.gtag_script_attributes, "id")]
                data-testid=[attr(&props.gtag_script_attributes, "data-testid")]
            {}
        }
        @if !props.skip_gtag_init {
            script
                type=(worker_type)
                defer
                id=[attr(&props.gtag_init_attributes, "id")]
                data-testid=[attr(&props.gtag_init_attributes, "data-testid")]
            {
                (gtag_init_script)
            }
        }
    }
}

#[must_use]
pub fn GoogleAnalyticsPartytown(props: &GoogleAnalyticsPartytownProps) -> Markup {
    GoogleAnalyticsPartytownHead(props)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_contains(html: &str, expected: &str) {
        assert!(html.contains(expected), "missing `{expected}` in `{html}`");
    }

    #[test]
    fn analytics_helpers_cover_defaults_custom_values_and_invalid_identifiers() {
        let props =
            DefaultGoogleAnalyticsPartytownProps(" G-NVR7S8D4JP ", " /~partytown/partytown.js ");
        assert!(props.should_render());
        assert_eq!(props.resolved_partytown_url(), "/~partytown/partytown.js");
        assert_eq!(props.resolved_worker_script_type(), "text/partytown");
        assert_eq!(props.resolved_data_layer_name(), "dataLayer");
        assert_eq!(props.resolved_gtag_function_name(), "gtag");
        assert_eq!(props.trimmed_gtag_id(), "G-NVR7S8D4JP");
        assert_eq!(props.resolved_forward(), vec!["dataLayer.push"]);
        assert_eq!(
            props.resolved_gtag_script_url(),
            "https://www.googletagmanager.com/gtag/js?id=G-NVR7S8D4JP"
        );
        assert!(is_js_identifier("$layer_1"));
        assert!(!is_js_identifier("1layer"));
        assert_eq!(encode_query("G A&B"), "G%20A%26B");

        let mut custom = GoogleAnalyticsPartytownProps::new("G CUSTOM", "/p.js");
        custom.gtag_script_base_url = "https://tags.example/script?debug=1".to_string();
        custom.data_layer_name = "customLayer".to_string();
        custom.gtag_function_name = "sendTag".to_string();
        custom.worker_script_type = "text/custom".to_string();
        custom.forward = vec![" ".to_string(), "customLayer.push".to_string()];
        custom
            .partytown_config
            .insert("debug".to_string(), json!(true));
        custom
            .gtag_config_params
            .insert("send_page_view".to_string(), json!(false));
        assert_eq!(custom.resolved_forward(), vec!["customLayer.push"]);
        assert_eq!(
            custom.resolved_gtag_script_url(),
            "https://tags.example/script?debug=1&id=G%20CUSTOM&l=customLayer"
        );
        assert_eq!(custom.resolved_worker_script_type(), "text/custom");
        assert_contains(
            &json_for_script(&json!("</script>&")),
            "\\u003c/script\\u003e\\u0026",
        );

        custom.data_layer_name = "bad-name".to_string();
        custom.gtag_function_name = "bad-name".to_string();
        assert_eq!(custom.resolved_data_layer_name(), "dataLayer");
        assert_eq!(custom.resolved_gtag_function_name(), "gtag");
        custom.disable_rendering = true;
        assert!(!custom.should_render());
    }

    #[test]
    fn analytics_renders_defaults_custom_skips_alias_and_escaping() {
        let html = GoogleAnalyticsPartytownHead(&DefaultGoogleAnalyticsPartytownProps(
            "G-NVR7S8D4JP",
            "/~partytown/partytown.js",
        ))
        .into_string();
        assert_contains(
            &html,
            r#"<link rel="preload" href="/~partytown/partytown.js" as="script">"#,
        );
        assert_contains(
            &html,
            r#"<script src="/~partytown/partytown.js" defer></script>"#,
        );
        assert_contains(
            &html,
            r#"window[&quot;dataLayer&quot;] = window[&quot;dataLayer&quot;] || [];"#,
        );
        assert_contains(&html, r#"&quot;forward&quot;:[&quot;dataLayer.push&quot;]"#);
        assert_contains(&html, r#"type="text/partytown""#);
        assert_contains(
            &html,
            r#"src="https://www.googletagmanager.com/gtag/js?id=G-NVR7S8D4JP""#,
        );
        assert_contains(
            &html,
            r#"window[&quot;gtag&quot;]('config', &quot;G-NVR7S8D4JP&quot;, {});"#,
        );

        let mut custom = GoogleAnalyticsPartytownProps::new(
            r#"G-ABC</script><script>alert("x")</script>"#,
            "/assets/partytown.js",
        );
        custom.data_layer_name = "customLayer".to_string();
        custom.gtag_function_name = "sendTag".to_string();
        custom.forward = vec!["customLayer.push".to_string(), "sendTag".to_string()];
        custom
            .partytown_config
            .insert("debug".to_string(), json!(true));
        custom
            .gtag_config_params
            .insert("groups".to_string(), json!("default"));
        custom
            .gtag_init_attributes
            .insert("data-testid".to_string(), "ga-init".to_string());
        let custom_html = GoogleAnalyticsPartytown(&custom).into_string();
        assert_contains(&custom_html, r#"href="/assets/partytown.js""#);
        assert_contains(&custom_html, r#"l=customLayer"#);
        assert_contains(&custom_html, r#"data-testid="ga-init""#);
        assert_contains(&custom_html, r#"\u003c/script\u003e"#);
        assert!(!custom_html.contains("</script><script>alert"));

        custom.skip_partytown_preload = true;
        custom.skip_partytown_script = true;
        custom.skip_partytown_config = true;
        custom.skip_gtag_script = true;
        custom.skip_gtag_init = true;
        assert_eq!(GoogleAnalyticsPartytownHead(&custom).into_string(), "");
        assert_eq!(
            GoogleAnalyticsPartytownHead(&GoogleAnalyticsPartytownProps::new("G", " "))
                .into_string(),
            ""
        );
    }
}
