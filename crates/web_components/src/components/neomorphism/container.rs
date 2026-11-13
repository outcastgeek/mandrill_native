#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::shared;
use crate::components::unstyled::container as base;
use maud::Markup;

pub use base::ContainerProps;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "neomorphism",
        module: "container",
        name: "Container",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "container",
        name: "Section",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "container",
        name: "Article",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "container",
        name: "Main",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "container",
        name: "FlatContainer",
    },
    ComponentInfo {
        family: "neomorphism",
        module: "container",
        name: "GlassContainer",
    },
];

#[must_use]
fn addNeomorphClasses(props: &ContainerProps) -> ContainerProps {
    let mut props = props.clone();
    props.class = shared::build_classes(&[props.class.as_str(), "neomorph-flat-sm"]);
    props
}

#[must_use]
pub fn Container(props: &ContainerProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Container(&neo_props, children)
}

#[must_use]
pub fn Section(props: &ContainerProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Section(&neo_props, children)
}

#[must_use]
pub fn Article(props: &ContainerProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Article(&neo_props, children)
}

#[must_use]
pub fn Main(props: &ContainerProps, children: Markup) -> Markup {
    let neo_props = addNeomorphClasses(props);
    base::Main(&neo_props, children)
}

#[must_use]
pub fn FlatContainer(
    maxWidth: &'static str,
    padding: &'static str,
    class: &str,
    children: Markup,
) -> Markup {
    Container(
        &ContainerProps::default()
            .with_max_width(maxWidth)
            .with_padding(padding)
            .with_centered(true)
            .with_class(class),
        children,
    )
}

#[must_use]
pub fn GlassContainer(
    maxWidth: &'static str,
    padding: &'static str,
    class: &str,
    children: Markup,
) -> Markup {
    Container(
        &ContainerProps::default()
            .with_max_width(maxWidth)
            .with_padding(padding)
            .with_centered(true)
            .with_class(shared::build_classes(&["neomorph-glass", class])),
        children,
    )
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
        assert_eq!(COMPONENTS.len(), 6);
        assert!(
            COMPONENTS
                .iter()
                .all(|component| component.family == "neomorphism"
                    && component.module == "container")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Container")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Section")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "Article")
        );
        assert!(COMPONENTS.iter().any(|component| component.name == "Main"));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "FlatContainer")
        );
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "GlassContainer")
        );
    }

    #[test]
    fn wrapped_components_render_unstyled_html_with_neomorphism_classes() {
        let html = Container(&ContainerProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("w-full max-w-screen-xl px-6 py-3 sm:px-8 sm:py-4 mx-auto"));
        assert!(html.contains("neomorph-flat-sm"));
        assert!(html.contains(">child</div>"));
        let html = Section(&ContainerProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<section"));
        assert!(html.contains("w-full max-w-screen-xl px-6 py-3 sm:px-8 sm:py-4 mx-auto"));
        assert!(html.contains("neomorph-flat-sm"));
        assert!(html.contains(">child</section>"));
        let html = Article(&ContainerProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<article"));
        assert!(html.contains("w-full max-w-screen-xl px-6 py-3 sm:px-8 sm:py-4 mx-auto"));
        assert!(html.contains("neomorph-flat-sm"));
        assert!(html.contains(">child</article>"));
        let html = Main(&ContainerProps::default(), html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<main"));
        assert!(html.contains("w-full max-w-screen-xl px-6 py-3 sm:px-8 sm:py-4 mx-auto"));
        assert!(html.contains("neomorph-flat-sm"));
        assert!(html.contains(">child</main>"));
    }

    #[test]
    fn neomorphism_only_components_render_html_without_generated_stub_markers() {
        let html =
            FlatContainer(shared::CONTAINER_MD, "sm", "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("max-w-screen-md"));
        assert!(html.contains("px-4 py-2"));
        assert!(html.contains("sample neomorph-flat-sm"));
        assert!(html.contains(">child</div>"));
        let html =
            GlassContainer(shared::CONTAINER_MD, "sm", "sample", html! { "child" }).into_string();
        assert_no_placeholder_markers(&html);
        assert!(html.starts_with("<div"));
        assert!(html.contains("max-w-screen-md"));
        assert!(html.contains("px-4 py-2"));
        assert!(html.contains("neomorph-glass sample neomorph-flat-sm"));
        assert!(html.contains(">child</div>"));
    }
}
