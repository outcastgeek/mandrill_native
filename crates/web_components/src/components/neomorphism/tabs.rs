#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::tabs as base;
use maud::Markup;

pub use base::{
    SimpleTabContent, SimpleTabList, SimpleTabTrigger, SimpleTabs, TAB_LIST_VARIANT_PILL,
    TAB_LIST_VARIANT_UNDERLINE, TabContent, TabContentProps, TabListProps, TabTriggerProps, Tabs,
    TabsProps, VerticalTabs,
};

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "tabs",
        name: "Tabs",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tabs",
        name: "TabList",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tabs",
        name: "TabTrigger",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tabs",
        name: "TabContent",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tabs",
        name: "SimpleTabs",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tabs",
        name: "VerticalTabs",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tabs",
        name: "SimpleTabList",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tabs",
        name: "SimpleTabTrigger",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "tabs",
        name: "SimpleTabContent",
    },
];

#[must_use]
fn addListNeomorphClasses(props: &TabListProps) -> TabListProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat"]);
    props
}

#[must_use]
fn addTriggerNeomorphClasses(props: &TabTriggerProps) -> TabTriggerProps {
    let mut props = props.clone();
    let neomorph_class = if props.disabled {
        ""
    } else {
        "neomorph-hover-lift data-[state=active]:neomorph-concave"
    };
    props.class = shared::build_classes(&[neomorph_class, props.class.as_str()]);
    props
}

#[must_use]
pub fn TabList(props: &TabListProps, children: Markup) -> Markup {
    let neo_props = addListNeomorphClasses(props);
    base::TabList(&neo_props, children)
}

#[must_use]
pub fn TabTrigger(props: &TabTriggerProps, children: Markup) -> Markup {
    let neo_props = addTriggerNeomorphClasses(props);
    base::TabTrigger(&neo_props, children)
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
        assert_eq!(COMPONENTS.len(), 9);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism" && component.module == "tabs")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Tabs"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TabList")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TabTrigger")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "TabContent")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleTabs")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "VerticalTabs")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleTabList")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleTabTrigger")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "SimpleTabContent")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = TabList(&TabListProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<"), "expected rendered element: {html}");
        assert!(
            html.contains("neomorph-flat"),
            "missing neomorph-flat: {html}"
        );

        let trigger_props = addTriggerNeomorphClasses(&TabTriggerProps::default());
        assert!(trigger_props.class.contains("neomorph-hover-lift"));
        assert!(
            trigger_props
                .class
                .contains("data-[state=active]:neomorph-concave")
        );
        let disabled = addTriggerNeomorphClasses(&TabTriggerProps::default().with_disabled(true));
        assert!(!disabled.class.contains("neomorph"));

        let trigger = TabTrigger(
            &TabTriggerProps::default().with_value("one"),
            html! { "One" },
        )
        .into_string();
        assert!(trigger.contains("neomorph-hover-lift"));
        assert!(trigger.contains(">One</button>"));
    }
}
