#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::scrollarea as base;
use maud::Markup;

pub use base::{
    SCROLL_ORIENTATION_BOTH, SCROLL_ORIENTATION_HORIZONTAL, SCROLL_ORIENTATION_VERTICAL,
    ScrollAreaProps,
};

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "neomorphism",
    module: "scrollarea",
    name: "ScrollArea",
}];

#[must_use]
fn addNeomorphClasses(props: &ScrollAreaProps) -> ScrollAreaProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn ScrollArea(props: &ScrollAreaProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::ScrollArea(&neo_props, children)
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
        assert_eq!(COMPONENTS.len(), 1);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "scrollarea")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "ScrollArea")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = ScrollArea(&ScrollAreaProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
