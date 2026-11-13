#![allow(non_snake_case)]

use crate::components::unstyled::navigationarrow::{NavigationArrow, NavigationArrowProps};
use crate::components::unstyled::paginationdots::{PaginationDots, PaginationDotsProps};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "carousel",
        name: "Carousel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "carousel",
        name: "SimpleCarousel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "carousel",
        name: "CarouselItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "carousel",
        name: "CarouselContent",
    },
    ComponentInfo {
        family: "unstyled",
        module: "carousel",
        name: "CarouselControl",
    },
    ComponentInfo {
        family: "unstyled",
        module: "carousel",
        name: "CarouselIndicators",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CarouselProps {
    pub items_per_view: i32,
    pub items_per_view_md: i32,
    pub items_per_view_sm: i32,
    pub gap: String,
    pub show_arrows: bool,
    pub show_pagination: bool,
    pub arrow_position: String,
    pub auto_play: bool,
    pub auto_play_delay: i32,
    pub pause_on_hover: bool,
    pub state_var: String,
    pub class: String,
    pub container_class: String,
    pub item_class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for CarouselProps {
    fn default() -> Self {
        Self {
            items_per_view: 3,
            items_per_view_md: 2,
            items_per_view_sm: 1,
            gap: "8".to_string(),
            show_arrows: true,
            show_pagination: true,
            arrow_position: "outside".to_string(),
            auto_play: false,
            auto_play_delay: 5000,
            pause_on_hover: true,
            state_var: "carouselState".to_string(),
            class: String::new(),
            container_class: String::new(),
            item_class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl CarouselProps {
    #[must_use]
    pub const fn with_auto_play(mut self, delay: i32) -> Self {
        self.auto_play = true;
        self.auto_play_delay = delay;
        self
    }
    #[must_use]
    pub const fn without_auto_play(mut self) -> Self {
        self.auto_play = false;
        self
    }
    #[must_use]
    pub const fn with_items_per_view(mut self, sm: i32, md: i32, lg: i32) -> Self {
        self.items_per_view_sm = sm;
        self.items_per_view_md = md;
        self.items_per_view = lg;
        self
    }
    #[must_use]
    pub const fn with_navigation(mut self, arrows: bool, pagination: bool) -> Self {
        self.show_arrows = arrows;
        self.show_pagination = pagination;
        self
    }
    #[must_use]
    pub fn with_state_var(mut self, value: impl Into<String>) -> Self {
        self.state_var = value.into();
        self
    }
    #[must_use]
    pub fn with_gap(mut self, value: impl Into<String>) -> Self {
        self.gap = value.into();
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_container_class(mut self, value: impl Into<String>) -> Self {
        self.container_class = value.into();
        self
    }
    #[must_use]
    pub fn with_item_class(mut self, value: impl Into<String>) -> Self {
        self.item_class = value.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn buildDatastarSignals(state_var: &str, total: i32) -> String {
    format!("{{{state_var}Current: 0, {state_var}Total: {total}}}")
}

#[must_use]
pub fn buildPrevClick(state_var: &str) -> String {
    format!(
        "${state_var}Current = (${state_var}Current - 1 + ${state_var}Total) % ${state_var}Total"
    )
}

#[must_use]
pub fn buildNextClick(state_var: &str) -> String {
    format!("${state_var}Current = (${state_var}Current + 1) % ${state_var}Total")
}

#[must_use]
pub fn buildItemVisibility(state_var: &str, index: i32) -> String {
    format!("${state_var}Current === {index}")
}

#[must_use]
pub fn indicatorClassExpression(state_var: &str, index: i32) -> String {
    format!(
        "${state_var}Current === {index} ? 'bg-primary' : 'bg-muted-foreground/30 hover:bg-muted-foreground/50'"
    )
}

#[must_use]
pub fn indicatorClickExpression(state_var: &str, index: i32) -> String {
    format!("${state_var}Current = {index}")
}

#[must_use]
pub fn Carousel(props: &CarouselProps, total_items: i32, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            class=(format!("carousel-wrapper {}", props.class))
            data-signals=(buildDatastarSignals(&props.state_var, total_items))
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            div class=(format!("relative overflow-hidden {}", props.container_class)) {
                div class="carousel-items" {
                    (children)
                }
            }
            @if props.show_arrows {
                div class="flex justify-center gap-4 mt-6" {
                    (NavigationArrow(&NavigationArrowProps::default()
                        .with_direction(shared::POSITION_LEFT)
                        .with_size(shared::SIZE_MD)
                        .with_on_click(buildPrevClick(&props.state_var))))
                    (NavigationArrow(&NavigationArrowProps::default()
                        .with_direction(shared::POSITION_RIGHT)
                        .with_size(shared::SIZE_MD)
                        .with_on_click(buildNextClick(&props.state_var))))
                }
            }
            @if props.show_pagination {
                (PaginationDots(&PaginationDotsProps::default()
                    .with_total(total_items)
                    .with_current(0)
                    .with_state_var(format!("{}Current", props.state_var))))
            }
        }
    }
}

#[must_use]
pub fn SimpleCarousel(total_items: i32, children: Markup) -> Markup {
    Carousel(&CarouselProps::default(), total_items, children)
}

#[must_use]
pub fn CarouselItem(index: i32, state_var: &str, class: &str, children: Markup) -> Markup {
    html! {
        div
            class=(shared::build_classes(&["carousel-item", class]))
            data-show=(buildItemVisibility(state_var, index))
            style="display: none"
        {
            (children)
        }
    }
}

#[must_use]
pub fn CarouselContent(class: &str, children: Markup) -> Markup {
    html! {
        div class=(shared::build_classes(&["relative overflow-hidden", class])) {
            (children)
        }
    }
}

#[must_use]
pub fn CarouselControl(direction: &str, state_var: &str, class: &str) -> Markup {
    let is_prev = direction == shared::CAROUSEL_PREV;
    html! {
        button
            type="button"
            class=(shared::build_classes(&[
                "inline-flex items-center justify-center h-10 w-10 rounded-full bg-background/80 backdrop-blur hover:bg-background transition-colors duration-150",
                class,
            ]))
            data-on:click=(if is_prev { buildPrevClick(state_var) } else { buildNextClick(state_var) })
            aria-label=(if is_prev { shared::default_strings().carousel_prev_slide } else { shared::default_strings().carousel_next_slide })
        {
            svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" {
                @if is_prev {
                    path d="m15 18-6-6 6-6" {}
                } @else {
                    path d="m9 18 6-6-6-6" {}
                }
            }
        }
    }
}

#[must_use]
pub fn CarouselIndicators(total: i32, state_var: &str, class: &str) -> Markup {
    html! {
        div class=(shared::build_classes(&["flex items-center justify-center gap-2 mt-4", class])) {
            @for index in 0..total {
                button
                    type="button"
                    class="w-2.5 h-2.5 rounded-full transition-colors duration-150"
                    data-class=(indicatorClassExpression(state_var, index))
                    data-on:click=(indicatorClickExpression(state_var, index))
                    aria-label=(format!("Go to slide {}", index + 1))
                {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_builders_and_datastar_expressions() {
        let props = CarouselProps::default();
        assert_eq!(props.items_per_view, 3);
        assert_eq!(props.items_per_view_md, 2);
        assert_eq!(props.items_per_view_sm, 1);
        assert_eq!(props.gap, "8");
        assert!(props.show_arrows);
        assert!(props.show_pagination);
        assert_eq!(props.arrow_position, "outside");
        assert!(!props.auto_play);
        assert_eq!(props.auto_play_delay, 5000);
        assert!(props.pause_on_hover);
        assert_eq!(props.state_var, "carouselState");
        assert_eq!(
            buildDatastarSignals("carouselState", 4),
            "{carouselStateCurrent: 0, carouselStateTotal: 4}"
        );
        assert_eq!(
            buildPrevClick("carouselState"),
            "$carouselStateCurrent = ($carouselStateCurrent - 1 + $carouselStateTotal) % $carouselStateTotal"
        );
        assert_eq!(
            buildNextClick("carouselState"),
            "$carouselStateCurrent = ($carouselStateCurrent + 1) % $carouselStateTotal"
        );
        assert_eq!(
            buildItemVisibility("carouselState", 2),
            "$carouselStateCurrent === 2"
        );
        assert_eq!(
            indicatorClickExpression("carouselState", 3),
            "$carouselStateCurrent = 3"
        );
        assert_eq!(
            indicatorClassExpression("carouselState", 1),
            "$carouselStateCurrent === 1 ? 'bg-primary' : 'bg-muted-foreground/30 hover:bg-muted-foreground/50'"
        );

        let built = CarouselProps::default()
            .with_auto_play(3000)
            .without_auto_play()
            .with_items_per_view(1, 3, 5)
            .with_navigation(false, true)
            .with_state_var("slides")
            .with_gap("4")
            .with_class("outer")
            .with_container_class("clip")
            .with_item_class("item")
            .with_attribute("data-testid", "carousel");
        assert!(!built.auto_play);
        assert_eq!(built.auto_play_delay, 3000);
        assert_eq!(built.items_per_view_sm, 1);
        assert_eq!(built.items_per_view_md, 3);
        assert_eq!(built.items_per_view, 5);
        assert!(!built.show_arrows);
        assert!(built.show_pagination);
        assert_eq!(built.state_var, "slides");
        assert_eq!(built.gap, "4");
        assert_eq!(built.class, "outer");
        assert_eq!(built.container_class, "clip");
        assert_eq!(built.item_class, "item");
        assert_eq!(
            built
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("carousel")
        );
    }

    #[test]
    fn carousel_renders_children_navigation_pagination_and_attributes() {
        let rendered = Carousel(
            &CarouselProps::default()
                .with_state_var("slides")
                .with_class("featured")
                .with_container_class("rounded")
                .with_attribute("id", "carousel")
                .with_attribute("data-testid", "carousel"),
            3,
            html! {
                (CarouselItem(0, "slides", "first", html! { "One" }))
                (CarouselItem(1, "slides", "", html! { "Two" }))
            },
        )
        .into_string();
        assert!(rendered.starts_with("<div class=\"carousel-wrapper featured\" data-signals=\"{slidesCurrent: 0, slidesTotal: 3}\" id=\"carousel\" data-testid=\"carousel\">"));
        assert!(rendered.contains(
            "<div class=\"relative overflow-hidden rounded\"><div class=\"carousel-items\">"
        ));
        assert!(rendered.contains("class=\"carousel-item first\" data-show=\"$slidesCurrent === 0\" style=\"display: none\">One</div>"));
        assert!(rendered.contains("class=\"carousel-item\" data-show=\"$slidesCurrent === 1\" style=\"display: none\">Two</div>"));
        assert!(rendered.contains("aria-label=\"Previous\""));
        assert!(rendered.contains(
            "data-on:click=\"$slidesCurrent = ($slidesCurrent - 1 + $slidesTotal) % $slidesTotal\""
        ));
        assert!(rendered.contains("aria-label=\"Next\""));
        assert!(
            rendered
                .contains("data-on:click=\"$slidesCurrent = ($slidesCurrent + 1) % $slidesTotal\"")
        );
        assert!(rendered.contains("role=\"tablist\" aria-label=\"Pagination\""));
        assert!(rendered.contains("data-on:click=\"$slidesCurrent = 2\""));
    }

    #[test]
    fn carousel_supports_hidden_navigation_and_simple_wrapper() {
        let rendered = Carousel(
            &CarouselProps::default().with_navigation(false, false),
            1,
            html! { "Only" },
        )
        .into_string();
        assert!(
            rendered.contains("data-signals=\"{carouselStateCurrent: 0, carouselStateTotal: 1}\"")
        );
        assert!(!rendered.contains("aria-label=\"Previous\""));
        assert!(!rendered.contains("role=\"tablist\""));
        assert!(rendered.contains("<div class=\"carousel-items\">Only</div>"));

        let simple = SimpleCarousel(2, html! { "Slides" }).into_string();
        assert!(simple.contains("class=\"carousel-wrapper \""));
        assert!(
            simple.contains("data-signals=\"{carouselStateCurrent: 0, carouselStateTotal: 2}\"")
        );
        assert!(simple.contains("Slides"));
    }

    #[test]
    fn helper_components_render_exact_controls_content_and_indicators() {
        assert_eq!(
            CarouselContent("clip", html! { "Body" }).into_string(),
            "<div class=\"relative overflow-hidden clip\">Body</div>",
        );
        let prev = CarouselControl(shared::CAROUSEL_PREV, "slides", "edge").into_string();
        assert!(prev.starts_with("<button type=\"button\" class=\"inline-flex items-center justify-center h-10 w-10 rounded-full"));
        assert!(prev.contains("edge\" data-on:click=\"$slidesCurrent = ($slidesCurrent - 1 + $slidesTotal) % $slidesTotal\" aria-label=\"Previous slide\""));
        assert!(prev.contains("<path d=\"m15 18-6-6 6-6\"></path>"));

        let next = CarouselControl(shared::CAROUSEL_NEXT, "slides", "").into_string();
        assert!(next.contains("data-on:click=\"$slidesCurrent = ($slidesCurrent + 1) % $slidesTotal\" aria-label=\"Next slide\""));
        assert!(next.contains("<path d=\"m9 18 6-6-6-6\"></path>"));

        let indicators = CarouselIndicators(2, "slides", "dots").into_string();
        assert!(
            indicators
                .starts_with("<div class=\"flex items-center justify-center gap-2 mt-4 dots\">")
        );
        assert_eq!(indicators.matches("<button ").count(), 2);
        assert!(indicators.contains("data-class=\"$slidesCurrent === 0 ? 'bg-primary' : 'bg-muted-foreground/30 hover:bg-muted-foreground/50'\""));
        assert!(indicators.contains("data-on:click=\"$slidesCurrent = 1\""));
        assert!(indicators.contains("aria-label=\"Go to slide 2\""));
    }
}
