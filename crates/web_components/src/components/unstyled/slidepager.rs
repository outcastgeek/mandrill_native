#![allow(non_snake_case)]

use crate::components::unstyled::paginationdots::{PaginationDots, PaginationDotsProps};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "slidepager",
        name: "SlidePager",
    },
    ComponentInfo {
        family: "unstyled",
        module: "slidepager",
        name: "SlidePanel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "slidepager",
        name: "SlideCounter",
    },
    ComponentInfo {
        family: "unstyled",
        module: "slidepager",
        name: "SlideNavigation",
    },
    ComponentInfo {
        family: "unstyled",
        module: "slidepager",
        name: "SlideThumbnailStrip",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlidePagerProps {
    pub total_slides: i32,
    pub state_var: String,
    pub show_counter: bool,
    pub show_thumbnails: bool,
    pub show_arrows: bool,
    pub show_dots: bool,
    pub enable_keyboard: bool,
    pub compact: bool,
    pub class: String,
    pub content_class: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlideCounterProps {
    pub state_var: String,
    pub total_slides: i32,
    pub class: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlideThumbnail {
    pub index: i32,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlideThumbnailStripProps {
    pub thumbnails: Vec<SlideThumbnail>,
    pub state_var: String,
    pub class: String,
}

impl SlidePagerProps {
    #[must_use]
    pub fn default_for(total_slides: i32) -> Self {
        Self {
            total_slides,
            state_var: "currentSlide".to_string(),
            show_counter: true,
            show_thumbnails: false,
            show_arrows: true,
            show_dots: false,
            enable_keyboard: true,
            compact: false,
            class: String::new(),
            content_class: String::new(),
            attributes: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn compact_for(total_slides: i32) -> Self {
        Self {
            compact: true,
            ..Self::default_for(total_slides)
        }
    }

    #[must_use]
    pub fn full_for(total_slides: i32) -> Self {
        Self {
            show_thumbnails: true,
            ..Self::default_for(total_slides)
        }
    }

    #[must_use]
    pub fn with_state_var(mut self, value: impl Into<String>) -> Self {
        self.state_var = value.into();
        self
    }
    #[must_use]
    pub const fn with_counter(mut self, value: bool) -> Self {
        self.show_counter = value;
        self
    }
    #[must_use]
    pub const fn with_thumbnails(mut self, value: bool) -> Self {
        self.show_thumbnails = value;
        self
    }
    #[must_use]
    pub const fn with_arrows(mut self, value: bool) -> Self {
        self.show_arrows = value;
        self
    }
    #[must_use]
    pub const fn with_dots(mut self, value: bool) -> Self {
        self.show_dots = value;
        self
    }
    #[must_use]
    pub const fn with_keyboard(mut self, value: bool) -> Self {
        self.enable_keyboard = value;
        self
    }
    #[must_use]
    pub const fn with_compact(mut self, value: bool) -> Self {
        self.compact = value;
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_content_class(mut self, value: impl Into<String>) -> Self {
        self.content_class = value.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
}

impl Default for SlidePagerProps {
    fn default() -> Self {
        Self::default_for(0)
    }
}

impl Default for SlideCounterProps {
    fn default() -> Self {
        Self {
            state_var: "currentSlide".to_string(),
            total_slides: 0,
            class: String::new(),
        }
    }
}

impl SlideCounterProps {
    #[must_use]
    pub fn with_state_var(mut self, value: impl Into<String>) -> Self {
        self.state_var = value.into();
        self
    }
    #[must_use]
    pub const fn with_total_slides(mut self, value: i32) -> Self {
        self.total_slides = value;
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
}

impl SlideThumbnail {
    #[must_use]
    pub fn new(index: i32, label: impl Into<String>) -> Self {
        Self {
            index,
            label: label.into(),
        }
    }
}

impl Default for SlideThumbnailStripProps {
    fn default() -> Self {
        Self {
            thumbnails: Vec::new(),
            state_var: "currentSlide".to_string(),
            class: String::new(),
        }
    }
}

impl SlideThumbnailStripProps {
    #[must_use]
    pub fn with_thumbnails(mut self, value: Vec<SlideThumbnail>) -> Self {
        self.thumbnails = value;
        self
    }
    #[must_use]
    pub fn with_state_var(mut self, value: impl Into<String>) -> Self {
        self.state_var = value.into();
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
}

#[must_use]
pub fn DefaultSlidePagerProps(total_slides: i32) -> SlidePagerProps {
    SlidePagerProps::default_for(total_slides)
}

#[must_use]
pub fn CompactSlidePagerProps(total_slides: i32) -> SlidePagerProps {
    SlidePagerProps::compact_for(total_slides)
}

#[must_use]
pub fn FullSlidePagerProps(total_slides: i32) -> SlidePagerProps {
    SlidePagerProps::full_for(total_slides)
}

#[must_use]
pub fn buildSignals(props: &SlidePagerProps) -> String {
    format!(
        "{{{}Current: 0, {}Total: {}}}",
        props.state_var, props.state_var, props.total_slides
    )
}

#[must_use]
pub fn buildPrevClick(state_var: &str) -> String {
    format!("${state_var}Current = Math.max(0, ${state_var}Current - 1)")
}

#[must_use]
pub fn buildNextClick(state_var: &str, _total: i32) -> String {
    format!("${state_var}Current = Math.min(${state_var}Total - 1, ${state_var}Current + 1)")
}

#[must_use]
pub fn buildKeydownHandler(state_var: &str, total: i32) -> String {
    let _ = total;
    format!(
        "if (evt.key === 'ArrowLeft') {{ evt.preventDefault(); ${state_var}Current = Math.max(0, ${state_var}Current - 1); }} if (evt.key === 'ArrowRight') {{ evt.preventDefault(); ${state_var}Current = Math.min(${state_var}Total - 1, ${state_var}Current + 1); }}"
    )
}

#[must_use]
pub fn buildWrapperClasses(props: &SlidePagerProps) -> String {
    let base = if props.compact {
        "slide-pager slide-pager-compact"
    } else {
        "slide-pager"
    };
    shared::build_classes(&[base, props.class.as_str()])
}

#[must_use]
pub const fn contentAreaClass(compact: bool) -> &'static str {
    if compact {
        "relative overflow-hidden max-h-[400px]"
    } else {
        "relative overflow-hidden"
    }
}

#[must_use]
pub const fn counterClass(compact: bool) -> &'static str {
    if compact { "px-2 py-1" } else { "px-4 py-2" }
}

#[must_use]
pub const fn navContainerClass(compact: bool) -> &'static str {
    if compact {
        "flex justify-center gap-2 mt-2"
    } else {
        "flex justify-center gap-4 mt-4"
    }
}

#[must_use]
pub const fn navArrowSize(compact: bool) -> &'static str {
    if compact {
        shared::SIZE_SM
    } else {
        shared::SIZE_MD
    }
}

#[must_use]
pub fn slideCounterExpression(props: &SlideCounterProps) -> String {
    format!(
        "`Slide ${{${}Current + 1}} of ${{${}Total}}`",
        props.state_var, props.state_var
    )
}

#[must_use]
pub fn SlidePager(props: &SlidePagerProps, children: Markup) -> Markup {
    html! {
        div
            class=(buildWrapperClasses(props))
            data-signals=(buildSignals(props))
            data-on:keydown__window=[if props.enable_keyboard && !props.compact { Some(buildKeydownHandler(&props.state_var, props.total_slides)) } else { None }]
            tabindex=[if props.enable_keyboard && props.compact { Some("0") } else { None }]
            data-on:keydown=[if props.enable_keyboard && props.compact { Some(buildKeydownHandler(&props.state_var, props.total_slides)) } else { None }]
            id=[shared::attr_value(&props.attributes, "id")]
            data-testid=[shared::attr_value(&props.attributes, "data-testid")]
        {
            @if props.show_counter {
                (SlideCounter(&SlideCounterProps::default()
                    .with_state_var(&props.state_var)
                    .with_total_slides(props.total_slides)
                    .with_class(counterClass(props.compact))))
            }
            div class=(shared::build_classes(&[contentAreaClass(props.compact), props.content_class.as_str()])) {
                (children.clone())
            }
            @if props.show_arrows {
                (SlideNavigation(&props.state_var, props.total_slides, props.compact))
            }
            @if props.show_dots {
                (PaginationDots(&PaginationDotsProps::default()
                    .with_total(props.total_slides)
                    .with_current(0)
                    .with_state_var(format!("{}Current", props.state_var))))
            }
            @if props.show_thumbnails {
                div class="mt-2" id=(format!("{}-thumbnails", props.state_var)) {
                    (children)
                }
            }
        }
    }
}

#[must_use]
pub fn SlidePanel(index: i32, state_var: &str, children: Markup) -> Markup {
    let expression = format!("${state_var}Current === {index}");
    html! {
        div
            data-show=(&expression)
            data-class:slide-active=(&expression)
            style="display: none"
            class="slide-panel"
        {
            (children)
        }
    }
}

#[must_use]
pub fn SlideCounter(props: &SlideCounterProps) -> Markup {
    html! {
        div class=(shared::build_classes(&["flex items-center justify-between", props.class.as_str()])) {
            span
                class="text-sm text-muted-foreground"
                data-text=(slideCounterExpression(props))
            {}
        }
    }
}

fn slide_nav_button(
    direction: &'static str,
    size: &'static str,
    on_click: String,
    disabled_expr: String,
) -> Markup {
    let label = if direction == shared::POSITION_LEFT {
        shared::default_strings().nav_arrow_previous
    } else {
        shared::default_strings().nav_arrow_next
    };
    let path = if direction == shared::POSITION_LEFT {
        "M15 19l-7-7 7-7"
    } else {
        "M9 5l7 7-7 7"
    };
    let size_class = match size {
        shared::SIZE_SM => "p-2",
        shared::SIZE_LG => "p-4",
        _ => "p-3",
    };
    html! {
        button
            type="button"
            class=(shared::build_classes(&[
                "rounded-full flex items-center justify-center transition-transform cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed",
                size_class,
            ]))
            aria-label=(label)
            data-on:click=(on_click)
            data-attr:disabled=(disabled_expr)
        {
            svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true" {
                path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d=(path) {}
            }
        }
    }
}

#[must_use]
pub fn SlideNavigation(state_var: &str, total: i32, compact: bool) -> Markup {
    html! {
        div class=(navContainerClass(compact)) {
            (slide_nav_button(
                shared::POSITION_LEFT,
                navArrowSize(compact),
                buildPrevClick(state_var),
                format!("${state_var}Current === 0"),
            ))
            (slide_nav_button(
                shared::POSITION_RIGHT,
                navArrowSize(compact),
                buildNextClick(state_var, total),
                format!("${state_var}Current === ${state_var}Total - 1"),
            ))
        }
    }
}

#[must_use]
pub fn thumbnailClassExpression(state_var: &str, index: i32) -> String {
    format!(
        "${state_var}Current === {index} ? 'border-primary bg-primary/10 text-primary' : 'border-border bg-muted/50 text-muted-foreground hover:border-primary/50'"
    )
}

#[must_use]
pub fn thumbnailClickExpression(state_var: &str, index: i32) -> String {
    format!("${state_var}Current = {index}")
}

#[must_use]
pub fn thumbnailAriaLabel(index: i32, label: &str) -> String {
    format!("Go to slide {}: {label}", index + 1)
}

#[must_use]
pub fn SlideThumbnailStrip(props: &SlideThumbnailStripProps) -> Markup {
    html! {
        div class=(shared::build_classes(&["flex gap-2 overflow-x-auto py-2 px-1 scrollbar-thin", props.class.as_str()])) {
            @for thumb in &props.thumbnails {
                button
                    type="button"
                    class="flex-shrink-0 w-16 h-12 rounded border-2 transition-colors duration-150 text-xs truncate px-1"
                    data-class=(thumbnailClassExpression(&props.state_var, thumb.index))
                    data-on:click=(thumbnailClickExpression(&props.state_var, thumb.index))
                    aria-label=(thumbnailAriaLabel(thumb.index, &thumb.label))
                {
                    (&thumb.label)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_and_helpers_cover_defaults_compact_full_and_classes() {
        assert_eq!(SlidePagerProps::default().total_slides, 0);
        let default = DefaultSlidePagerProps(5);
        assert_eq!(default.total_slides, 5);
        assert_eq!(default.state_var, "currentSlide");
        assert!(default.show_counter);
        assert!(default.show_arrows);
        assert!(!default.show_dots);
        assert!(!default.show_thumbnails);
        assert!(default.enable_keyboard);
        assert!(!default.compact);
        assert_eq!(
            buildSignals(&default),
            "{currentSlideCurrent: 0, currentSlideTotal: 5}"
        );
        assert_eq!(buildWrapperClasses(&default), "slide-pager");
        assert_eq!(contentAreaClass(false), "relative overflow-hidden");
        assert_eq!(counterClass(false), "px-4 py-2");
        assert_eq!(navContainerClass(false), "flex justify-center gap-4 mt-4");
        assert_eq!(navArrowSize(false), shared::SIZE_MD);

        let compact = CompactSlidePagerProps(3);
        assert!(compact.compact);
        assert_eq!(
            buildWrapperClasses(&compact),
            "slide-pager slide-pager-compact"
        );
        assert_eq!(
            contentAreaClass(true),
            "relative overflow-hidden max-h-[400px]"
        );
        assert_eq!(counterClass(true), "px-2 py-1");
        assert_eq!(navContainerClass(true), "flex justify-center gap-2 mt-2");
        assert_eq!(navArrowSize(true), shared::SIZE_SM);

        let full = FullSlidePagerProps(7);
        assert!(full.show_thumbnails);

        let built = DefaultSlidePagerProps(2)
            .with_state_var("deck")
            .with_counter(false)
            .with_thumbnails(true)
            .with_arrows(false)
            .with_dots(true)
            .with_keyboard(false)
            .with_compact(true)
            .with_class("outer")
            .with_content_class("content")
            .with_attribute("data-testid", "deck");
        assert_eq!(built.state_var, "deck");
        assert!(!built.show_counter);
        assert!(built.show_thumbnails);
        assert!(!built.show_arrows);
        assert!(built.show_dots);
        assert!(!built.enable_keyboard);
        assert!(built.compact);
        assert_eq!(built.class, "outer");
        assert_eq!(built.content_class, "content");
        assert_eq!(
            built.attributes.get("data-testid").map(String::as_str),
            Some("deck")
        );
    }

    #[test]
    fn expression_helpers_match_datastar_output() {
        assert_eq!(
            buildPrevClick("deck"),
            "$deckCurrent = Math.max(0, $deckCurrent - 1)"
        );
        assert_eq!(
            buildNextClick("deck", 3),
            "$deckCurrent = Math.min($deckTotal - 1, $deckCurrent + 1)"
        );
        assert_eq!(
            buildKeydownHandler("deck", 3),
            "if (evt.key === 'ArrowLeft') { evt.preventDefault(); $deckCurrent = Math.max(0, $deckCurrent - 1); } if (evt.key === 'ArrowRight') { evt.preventDefault(); $deckCurrent = Math.min($deckTotal - 1, $deckCurrent + 1); }",
        );
        assert_eq!(
            slideCounterExpression(&SlideCounterProps::default().with_state_var("deck")),
            "`Slide ${$deckCurrent + 1} of ${$deckTotal}`"
        );
        assert_eq!(
            thumbnailClassExpression("deck", 1),
            "$deckCurrent === 1 ? 'border-primary bg-primary/10 text-primary' : 'border-border bg-muted/50 text-muted-foreground hover:border-primary/50'"
        );
        assert_eq!(thumbnailClickExpression("deck", 2), "$deckCurrent = 2");
        assert_eq!(thumbnailAriaLabel(2, "Intro"), "Go to slide 3: Intro");
    }

    #[test]
    fn slide_panel_counter_navigation_and_thumbnails_render_html() {
        let panel = SlidePanel(2, "galleryDeck", html! { "<Slide>" }).into_string();
        assert_eq!(
            panel,
            "<div data-show=\"$galleryDeckCurrent === 2\" data-class:slide-active=\"$galleryDeckCurrent === 2\" style=\"display: none\" class=\"slide-panel\">&lt;Slide&gt;</div>",
        );

        let counter = SlideCounter(
            &SlideCounterProps::default()
                .with_state_var("deck")
                .with_class("px"),
        )
        .into_string();
        assert_eq!(
            counter,
            "<div class=\"flex items-center justify-between px\"><span class=\"text-sm text-muted-foreground\" data-text=\"`Slide ${$deckCurrent + 1} of ${$deckTotal}`\"></span></div>",
        );

        let nav = SlideNavigation("deck", 3, true).into_string();
        assert!(nav.starts_with("<div class=\"flex justify-center gap-2 mt-2\">"));
        assert!(nav.contains("aria-label=\"Previous\" data-on:click=\"$deckCurrent = Math.max(0, $deckCurrent - 1)\" data-attr:disabled=\"$deckCurrent === 0\""));
        assert!(nav.contains("aria-label=\"Next\" data-on:click=\"$deckCurrent = Math.min($deckTotal - 1, $deckCurrent + 1)\" data-attr:disabled=\"$deckCurrent === $deckTotal - 1\""));
        assert!(
            slide_nav_button(
                shared::POSITION_LEFT,
                shared::SIZE_LG,
                "$deckCurrent = 0".to_string(),
                "false".to_string(),
            )
            .into_string()
            .contains("p-4")
        );

        let thumbs = SlideThumbnailStrip(
            &SlideThumbnailStripProps::default()
                .with_state_var("deck")
                .with_class("strip")
                .with_thumbnails(vec![
                    SlideThumbnail::new(0, "<One>"),
                    SlideThumbnail::new(1, "Two"),
                ]),
        )
        .into_string();
        assert!(thumbs.starts_with(
            "<div class=\"flex gap-2 overflow-x-auto py-2 px-1 scrollbar-thin strip\">"
        ));
        assert_eq!(thumbs.matches("<button ").count(), 2);
        assert!(thumbs.contains("data-class=\"$deckCurrent === 0 ? 'border-primary bg-primary/10 text-primary' : 'border-border bg-muted/50 text-muted-foreground hover:border-primary/50'\""));
        assert!(
            thumbs.contains("data-on:click=\"$deckCurrent = 1\" aria-label=\"Go to slide 2: Two\"")
        );
        assert!(thumbs.contains("&lt;One&gt;"));
    }

    #[test]
    fn slide_pager_renders_global_keyboard_compact_keyboard_controls_dots_and_thumbnails() {
        let full = SlidePager(
            &FullSlidePagerProps(2)
                .with_state_var("deck")
                .with_dots(true)
                .with_class("outer")
                .with_content_class("content")
                .with_attribute("id", "deck-root"),
            html! { (SlidePanel(0, "deck", html! { "A" })) },
        )
        .into_string();
        assert!(full.starts_with("<div class=\"slide-pager outer\" data-signals=\"{deckCurrent: 0, deckTotal: 2}\" data-on:keydown__window=\"if (evt.key === 'ArrowLeft')"));
        assert!(full.contains("id=\"deck-root\""));
        assert!(full.contains("<div class=\"flex items-center justify-between px-4 py-2\">"));
        assert!(full.contains("<div class=\"relative overflow-hidden content\">"));
        assert!(full.contains("data-attr:disabled=\"$deckCurrent === 0\""));
        assert!(full.contains("role=\"tablist\" aria-label=\"Pagination\""));
        assert!(full.contains("<div class=\"mt-2\" id=\"deck-thumbnails\">"));

        let compact = SlidePager(
            &CompactSlidePagerProps(1).with_state_var("mini"),
            html! { "Mini" },
        )
        .into_string();
        assert!(compact.starts_with("<div class=\"slide-pager slide-pager-compact\" data-signals=\"{miniCurrent: 0, miniTotal: 1}\" tabindex=\"0\" data-on:keydown=\"if (evt.key === 'ArrowLeft')"));
        assert!(!compact.contains("data-on:keydown__window"));
        assert!(compact.contains("<div class=\"relative overflow-hidden max-h-[400px]\">"));

        let quiet = SlidePager(
            &DefaultSlidePagerProps(1)
                .with_counter(false)
                .with_arrows(false)
                .with_keyboard(false),
            html! { "Only" },
        )
        .into_string();
        assert!(!quiet.contains("data-on:keydown"));
        assert!(!quiet.contains("text-muted-foreground"));
        assert!(!quiet.contains("aria-label=\"Previous\""));
        assert!(quiet.contains("Only"));
    }
}
