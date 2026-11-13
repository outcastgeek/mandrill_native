#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::slidepager as base;
use maud::Markup;

pub use base::{
    SlideCounterProps, SlideNavigation, SlidePagerProps, SlidePanel, SlideThumbnail,
    SlideThumbnailStripProps,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "slidepager",
        name: "SlidePager",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "slidepager",
        name: "SlidePanel",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "slidepager",
        name: "SlideCounter",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "slidepager",
        name: "SlideNavigation",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "slidepager",
        name: "SlideThumbnailStrip",
    },
];

#[must_use]
fn addPagerNeomorphClasses(props: &SlidePagerProps) -> SlidePagerProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn SlidePager(props: &SlidePagerProps, children: Markup) -> Markup {
    let neo_props = addPagerNeomorphClasses(props);
    base::SlidePager(&neo_props, children)
}

#[must_use]
fn addCounterNeomorphClasses(props: &SlideCounterProps) -> SlideCounterProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn SlideCounter(props: &SlideCounterProps) -> Markup {
    let neo_props = addCounterNeomorphClasses(props);
    base::SlideCounter(&neo_props)
}

#[must_use]
fn addThumbnailNeomorphClasses(props: &SlideThumbnailStripProps) -> SlideThumbnailStripProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
pub fn SlideThumbnailStrip(props: &SlideThumbnailStripProps) -> Markup {
    let neo_props = addThumbnailNeomorphClasses(props);
    base::SlideThumbnailStrip(&neo_props)
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
        assert_eq!(COMPONENTS.len(), 5);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "slidepager")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SlidePager")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SlidePanel")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SlideCounter")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SlideNavigation")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SlideThumbnailStrip")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = SlidePager(&SlidePagerProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = SlideCounter(&SlideCounterProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
        let html = SlideThumbnailStrip(&SlideThumbnailStripProps::default()).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );
    }
}
