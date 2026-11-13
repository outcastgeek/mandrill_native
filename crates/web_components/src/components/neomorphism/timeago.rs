#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::timeago as base;
use maud::Markup;

pub use base::{SimpleTimeAgo, TimeAgoProps};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "timeago",
        name: "TimeAgo",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "timeago",
        name: "SimpleTimeAgo",
    },
];

#[must_use]
fn addNeomorphClasses(props: &TimeAgoProps) -> TimeAgoProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn TimeAgo(props: &TimeAgoProps) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::TimeAgo(&neo_props)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn assert_no_placeholder_markers(html: &str) {
        assert!(
            !html.contains("data-family="),
            "placeholder family marker leaked: {html}"
        );
        assert!(
            !html.contains("data-module="),
            "placeholder module marker leaked: {html}"
        );
        assert!(
            !html.contains("data-component=\""),
            "placeholder component marker leaked: {html}"
        );
    }

    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 2);
        assert!(COMPONENTS.iter().all(|component| component.family == "neomorphism" && component.module == "timeago"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TimeAgo")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleTimeAgo")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = TimeAgo(
            &TimeAgoProps::default()
                .with_datetime("2026-01-01T00:00:00Z")
                .with_display("now"),
        )
        .into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
