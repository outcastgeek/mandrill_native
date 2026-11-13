#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::activityfeed as base;
use maud::Markup;

pub use base::{
    ActivityFeedProps, ActivityItemProps, CONNECTED, CONNECTING, DISCONNECTED, SimpleActivityFeed,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "activityfeed",
        name: "ActivityItem",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "activityfeed",
        name: "ActivityFeed",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "activityfeed",
        name: "SimpleActivityFeed",
    },
];

#[must_use]
fn addItemNeomorphClasses(props: &ActivityItemProps) -> ActivityItemProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn ActivityItem(props: &ActivityItemProps) -> Markup {
    let neo_props = addItemNeomorphClasses(props);
    base::ActivityItem(&neo_props)
}

#[must_use]
fn addFeedNeomorphClasses(props: &ActivityFeedProps) -> ActivityFeedProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn ActivityFeed(props: &ActivityFeedProps, children: Markup) -> Markup {
    let neo_props = addFeedNeomorphClasses(props);
    base::ActivityFeed(&neo_props, children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maud::html;
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
        assert_eq!(COMPONENTS.len(), 3);
        assert!(COMPONENTS.iter().all(
            |component| component.family == "neomorphism" && component.module == "activityfeed"
        ));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ActivityItem")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ActivityFeed")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleActivityFeed")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = ActivityItem(&ActivityItemProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = ActivityFeed(&ActivityFeedProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
