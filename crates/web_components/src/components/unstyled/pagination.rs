#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "pagination",
        name: "PaginationItem",
    },
    ComponentInfo {
        family: "unstyled",
        module: "pagination",
        name: "Pagination",
    },
    ComponentInfo {
        family: "unstyled",
        module: "pagination",
        name: "ListPaginationBar",
    },
    ComponentInfo {
        family: "unstyled",
        module: "pagination",
        name: "paginationRoot",
    },
    ComponentInfo {
        family: "unstyled",
        module: "pagination",
        name: "listPaginationBarRoot",
    },
    ComponentInfo {
        family: "unstyled",
        module: "pagination",
        name: "paginationCompact",
    },
    ComponentInfo {
        family: "unstyled",
        module: "pagination",
        name: "paginationFull",
    },
    ComponentInfo {
        family: "unstyled",
        module: "pagination",
        name: "SimplePagination",
    },
    ComponentInfo {
        family: "unstyled",
        module: "pagination",
        name: "chevronLeft",
    },
    ComponentInfo {
        family: "unstyled",
        module: "pagination",
        name: "chevronRight",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaginationItemProps {
    pub page: i32,
    pub href: String,
    pub is_active: bool,
    pub is_disabled: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for PaginationItemProps {
    fn default() -> Self {
        Self {
            page: 1,
            href: String::new(),
            is_active: false,
            is_disabled: false,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl PaginationItemProps {
    #[must_use]
    pub const fn with_page(mut self, value: i32) -> Self {
        self.page = value;
        self
    }
    #[must_use]
    pub fn with_href(mut self, value: impl Into<String>) -> Self {
        self.href = value.into();
        self
    }
    #[must_use]
    pub const fn with_active(mut self, value: bool) -> Self {
        self.is_active = value;
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, value: bool) -> Self {
        self.is_disabled = value;
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        let mut base = "inline-flex items-center justify-center min-w-[2.5rem] h-10 px-2 text-sm font-medium rounded-md border border-transparent select-none transition-all duration-150".to_string();
        if self.is_active {
            base.push_str(
                " bg-primary text-primary-foreground scale-105 shadow-[0_2px_8px_rgba(0,0,0,0.15)]",
            );
        } else if self.is_disabled {
            base.push_str(
                " text-muted-foreground cursor-not-allowed opacity-40 pointer-events-none",
            );
        } else {
            base.push_str(" hover:bg-accent hover:text-accent-foreground hover:-translate-y-px active:scale-[0.97] active:translate-y-0");
        }
        shared::build_classes(&[base.as_str(), self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaginationProps {
    pub current_page: i32,
    pub total_pages: i32,
    pub base_href: String,
    pub show_first: bool,
    pub show_last: bool,
    pub show_prev_next: bool,
    pub max_visible: i32,
    pub compact: bool,
    pub responsive: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for PaginationProps {
    fn default() -> Self {
        Self {
            current_page: 1,
            total_pages: 1,
            base_href: String::new(),
            show_first: true,
            show_last: true,
            show_prev_next: true,
            max_visible: 5,
            compact: false,
            responsive: false,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl PaginationProps {
    #[must_use]
    pub fn normalize(&self) -> Self {
        let mut props = self.clone();
        if props.current_page <= 0 {
            props.current_page = 1;
        }
        if props.total_pages <= 0 {
            props.total_pages = 1;
        }
        if props.max_visible <= 0 {
            props.max_visible = 5;
        }
        if !props.show_first && !props.show_last && !props.show_prev_next {
            props.show_first = true;
            props.show_last = true;
            props.show_prev_next = true;
        }
        props
    }
    #[must_use]
    pub const fn with_current_page(mut self, value: i32) -> Self {
        self.current_page = value;
        self
    }
    #[must_use]
    pub const fn with_total_pages(mut self, value: i32) -> Self {
        self.total_pages = value;
        self
    }
    #[must_use]
    pub fn with_base_href(mut self, value: impl Into<String>) -> Self {
        self.base_href = value.into();
        self
    }
    #[must_use]
    pub const fn with_show_first(mut self, value: bool) -> Self {
        self.show_first = value;
        self
    }
    #[must_use]
    pub const fn with_show_last(mut self, value: bool) -> Self {
        self.show_last = value;
        self
    }
    #[must_use]
    pub const fn with_show_prev_next(mut self, value: bool) -> Self {
        self.show_prev_next = value;
        self
    }
    #[must_use]
    pub const fn with_max_visible(mut self, value: i32) -> Self {
        self.max_visible = value;
        self
    }
    #[must_use]
    pub const fn with_compact(mut self, value: bool) -> Self {
        self.compact = value;
        self
    }
    #[must_use]
    pub const fn with_responsive(mut self, value: bool) -> Self {
        self.responsive = value;
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["flex items-center gap-1", self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.normalize().attributes;
        attrs
            .entry("data-smooth-preserve-scroll".to_string())
            .or_insert_with(|| "true".to_string());
        attrs
    }
    #[must_use]
    pub fn has_prev(&self) -> bool {
        self.normalize().current_page > 1
    }
    #[must_use]
    pub fn has_next(&self) -> bool {
        let props = self.normalize();
        props.current_page < props.total_pages
    }
    #[must_use]
    pub fn get_page_range(&self) -> Vec<i32> {
        let props = self.normalize();
        if props.total_pages <= props.max_visible {
            return (1..=props.total_pages).collect();
        }
        let half = props.max_visible / 2;
        let mut start = props.current_page - half;
        let mut end = start + props.max_visible - 1;
        if start < 1 {
            start = 1;
            end = props.max_visible;
        }
        if end > props.total_pages {
            end = props.total_pages;
            start = props.total_pages - props.max_visible + 1;
        }
        (start..=end).collect()
    }
    #[must_use]
    pub fn first_visible_page(&self) -> i32 {
        self.get_page_range().first().copied().unwrap_or(0)
    }
    #[must_use]
    pub fn last_visible_page(&self) -> i32 {
        self.get_page_range().last().copied().unwrap_or(0)
    }
    #[must_use]
    pub fn contains_visible_page(&self, page: i32) -> bool {
        self.get_page_range().contains(&page)
    }
    #[must_use]
    pub fn should_render_first_page_link(&self) -> bool {
        let props = self.normalize();
        props.show_first && props.total_pages > 1 && !props.contains_visible_page(1)
    }
    #[must_use]
    pub fn should_render_last_page_link(&self) -> bool {
        let props = self.normalize();
        props.show_last && props.total_pages > 1 && !props.contains_visible_page(props.total_pages)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimplePaginationProps {
    pub current_page: i32,
    pub total_pages: i32,
    pub prev_href: String,
    pub next_href: String,
    pub prev_attributes: BTreeMap<String, String>,
    pub next_attributes: BTreeMap<String, String>,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for SimplePaginationProps {
    fn default() -> Self {
        Self {
            current_page: 1,
            total_pages: 1,
            prev_href: String::new(),
            next_href: String::new(),
            prev_attributes: BTreeMap::new(),
            next_attributes: BTreeMap::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl SimplePaginationProps {
    #[must_use]
    pub const fn with_current_page(mut self, value: i32) -> Self {
        self.current_page = value;
        self
    }
    #[must_use]
    pub const fn with_total_pages(mut self, value: i32) -> Self {
        self.total_pages = value;
        self
    }
    #[must_use]
    pub fn with_prev_href(mut self, value: impl Into<String>) -> Self {
        self.prev_href = value.into();
        self
    }
    #[must_use]
    pub fn with_next_href(mut self, value: impl Into<String>) -> Self {
        self.next_href = value.into();
        self
    }
    #[must_use]
    pub fn with_prev_attribute(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.prev_attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn with_next_attribute(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.next_attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "flex items-center justify-between gap-4",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs
            .entry("data-smooth-preserve-scroll".to_string())
            .or_insert_with(|| "true".to_string());
        attrs
    }
    #[must_use]
    pub fn build_prev_attributes(&self) -> BTreeMap<String, String> {
        self.prev_attributes.clone()
    }
    #[must_use]
    pub fn build_next_attributes(&self) -> BTreeMap<String, String> {
        self.next_attributes.clone()
    }
    #[must_use]
    pub const fn has_prev(&self) -> bool {
        self.current_page > 1
    }
    #[must_use]
    pub const fn has_next(&self) -> bool {
        self.current_page < self.total_pages
    }
}

#[must_use]
pub fn compactNavClasses(disabled: bool) -> String {
    let base = "inline-flex items-center gap-1.5 px-3 py-2 text-sm font-medium rounded-md border border-border min-h-[2.75rem]";
    if disabled {
        format!("{base} text-muted-foreground opacity-40 cursor-not-allowed pointer-events-none")
    } else {
        format!(
            "{base} hover:bg-accent hover:text-accent-foreground hover:border-accent transition-all duration-150"
        )
    }
}

#[must_use]
pub fn fullNavClasses(disabled: bool) -> String {
    let base =
        "inline-flex items-center justify-center min-w-[2.5rem] h-10 px-2 text-sm font-medium";
    if disabled {
        format!("{base} text-muted-foreground opacity-40 cursor-not-allowed pointer-events-none")
    } else {
        format!(
            "{base} rounded-md hover:bg-accent hover:text-accent-foreground hover:-translate-y-px active:scale-[0.97] active:translate-y-0 transition-all duration-150 pagination-page"
        )
    }
}

#[must_use]
pub fn buildPageHref(base_href: &str, page: i32) -> String {
    if base_href.is_empty() {
        return format!("?page={page}");
    }
    let page = page.to_string();
    base_href
        .replace("{page}", &page)
        .replace("%7Bpage%7D", &page)
        .replace("%7bpage%7d", &page)
}

#[must_use]
pub fn PaginationItem(props: &PaginationItemProps) -> Markup {
    let attrs = props.build_attributes();
    if props.is_disabled {
        html! {
            span
                class=(props.build_classes())
                aria-disabled="true"
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                (props.page)
            }
        }
    } else {
        html! {
            a
                href=(&props.href)
                class=(props.build_classes())
                aria-current=[if props.is_active { Some("page") } else { None }]
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                (props.page)
            }
        }
    }
}

#[must_use]
pub fn Pagination(props: &PaginationProps) -> Markup {
    paginationRoot(&props.normalize())
}

#[must_use]
pub fn ListPaginationBar(props: &PaginationProps) -> Markup {
    listPaginationBarRoot(&props.normalize())
}

#[must_use]
pub fn paginationRoot(props: &PaginationProps) -> Markup {
    if props.total_pages <= 1 {
        return html! {};
    }
    let attrs = props.build_attributes();
    if props.responsive {
        html! {
            nav
                aria-label=(shared::default_strings().pagination_aria_label)
                data-smooth-preserve-scroll=[shared::attr_value(&attrs, "data-smooth-preserve-scroll")]
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                div class=(format!("{} pagination-full", props.build_classes())) {
                    (paginationFull(props))
                }
                div class=(format!("{} pagination-compact", props.build_classes())) {
                    (paginationCompact(props))
                }
            }
        }
    } else {
        html! {
            nav
                class=(props.build_classes())
                aria-label=(shared::default_strings().pagination_aria_label)
                data-smooth-preserve-scroll=[shared::attr_value(&attrs, "data-smooth-preserve-scroll")]
                id=[shared::attr_value(&attrs, "id")]
                data-testid=[shared::attr_value(&attrs, "data-testid")]
            {
                @if props.compact {
                    (paginationCompact(props))
                } @else {
                    (paginationFull(props))
                }
            }
        }
    }
}

#[must_use]
pub fn listPaginationBarRoot(props: &PaginationProps) -> Markup {
    if props.total_pages <= 1 {
        html! {}
    } else {
        let class = shared::build_classes(&["justify-center lg:justify-end", props.class.as_str()]);
        paginationRoot(&props.clone().with_class(class).normalize())
    }
}

#[must_use]
pub fn paginationCompact(props: &PaginationProps) -> Markup {
    let previous_label = shared::default_strings().pagination_previous;
    let next_label = shared::default_strings().pagination_next;
    html! {
        @if props.has_prev() {
            a
                href=(buildPageHref(&props.base_href, props.current_page - 1))
                class=(compactNavClasses(false))
                aria-label=(shared::default_strings().pagination_previous_page)
            {
                (chevronLeft())
                (previous_label)
            }
        } @else {
            span class=(compactNavClasses(true)) {
                (chevronLeft())
                (previous_label)
            }
        }
        span class="text-sm font-medium text-muted-foreground" {
            (format!("Page {} of {}", props.current_page, props.total_pages))
        }
        @if props.has_next() {
            a
                href=(buildPageHref(&props.base_href, props.current_page + 1))
                class=(compactNavClasses(false))
                aria-label=(shared::default_strings().pagination_next_page)
            {
                (next_label)
                (chevronRight())
            }
        } @else {
            span class=(compactNavClasses(true)) {
                (next_label)
                (chevronRight())
            }
        }
    }
}

#[must_use]
pub fn paginationFull(props: &PaginationProps) -> Markup {
    html! {
        @if props.show_prev_next {
            @if props.has_prev() {
                a href=(buildPageHref(&props.base_href, props.current_page - 1)) class=(fullNavClasses(false)) aria-label=(shared::default_strings().pagination_previous_page) {
                    (chevronLeft())
                }
            } @else {
                span class=(fullNavClasses(true)) {
                    (chevronLeft())
                }
            }
        }
        @if props.should_render_first_page_link() {
            (PaginationItem(&PaginationItemProps::default().with_page(1).with_href(buildPageHref(&props.base_href, 1))))
            @if props.first_visible_page() > 2 {
                span class="inline-flex items-center justify-center min-w-[2.5rem] h-10 text-sm text-muted-foreground tracking-wider" { "..." }
            }
        }
        @for page in props.get_page_range() {
            (PaginationItem(&PaginationItemProps::default()
                .with_page(page)
                .with_href(buildPageHref(&props.base_href, page))
                .with_active(page == props.current_page)))
        }
        @if props.should_render_last_page_link() {
            @if props.last_visible_page() < props.total_pages - 1 {
                span class="inline-flex items-center justify-center min-w-[2.5rem] h-10 text-sm text-muted-foreground tracking-wider" { "..." }
            }
            (PaginationItem(&PaginationItemProps::default().with_page(props.total_pages).with_href(buildPageHref(&props.base_href, props.total_pages))))
        }
        @if props.show_prev_next {
            @if props.has_next() {
                a href=(buildPageHref(&props.base_href, props.current_page + 1)) class=(fullNavClasses(false)) aria-label=(shared::default_strings().pagination_next_page) {
                    (chevronRight())
                }
            } @else {
                span class=(fullNavClasses(true)) {
                    (chevronRight())
                }
            }
        }
    }
}

fn attr_id(attrs: &BTreeMap<String, String>) -> Option<&str> {
    shared::attr_value(attrs, "id")
}
fn attr_testid(attrs: &BTreeMap<String, String>) -> Option<&str> {
    shared::attr_value(attrs, "data-testid")
}
fn attr_on_click(attrs: &BTreeMap<String, String>) -> Option<&str> {
    shared::attr_value(attrs, "data-on:click")
}

#[must_use]
pub fn SimplePagination(props: &SimplePaginationProps) -> Markup {
    if props.total_pages <= 1 {
        return html! {};
    }
    let attrs = props.build_attributes();
    let prev_attrs = props.build_prev_attributes();
    let next_attrs = props.build_next_attributes();
    html! {
        nav
            class=(props.build_classes())
            aria-label=(shared::default_strings().pagination_aria_label)
            data-smooth-preserve-scroll=[shared::attr_value(&attrs, "data-smooth-preserve-scroll")]
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            @if props.has_prev() {
                @if !props.prev_href.trim().is_empty() {
                    a
                        href=(&props.prev_href)
                        class="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-lg hover:bg-accent hover:text-accent-foreground transition-all duration-150"
                        id=[attr_id(&prev_attrs)]
                        data-testid=[attr_testid(&prev_attrs)]
                        data-on:click=[attr_on_click(&prev_attrs)]
                    {
                        (chevronLeft())
                        span class="pagination-label" { (shared::default_strings().pagination_previous) }
                    }
                } @else {
                    button
                        type="button"
                        class="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-lg hover:bg-accent hover:text-accent-foreground transition-all duration-150"
                        id=[attr_id(&prev_attrs)]
                        data-testid=[attr_testid(&prev_attrs)]
                        data-on:click=[attr_on_click(&prev_attrs)]
                    {
                        (chevronLeft())
                        span class="pagination-label" { (shared::default_strings().pagination_previous) }
                    }
                }
            } @else {
                span class="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-muted-foreground opacity-50" {
                    (chevronLeft())
                    span class="pagination-label" { (shared::default_strings().pagination_previous) }
                }
            }
            span class="text-sm text-muted-foreground whitespace-nowrap" {
                (format!("Page {} of {}", props.current_page, props.total_pages))
            }
            @if props.has_next() {
                @if !props.next_href.trim().is_empty() {
                    a
                        href=(&props.next_href)
                        class="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-lg hover:bg-accent hover:text-accent-foreground transition-all duration-150"
                        id=[attr_id(&next_attrs)]
                        data-testid=[attr_testid(&next_attrs)]
                        data-on:click=[attr_on_click(&next_attrs)]
                    {
                        span class="pagination-label" { (shared::default_strings().pagination_next) }
                        (chevronRight())
                    }
                } @else {
                    button
                        type="button"
                        class="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-lg hover:bg-accent hover:text-accent-foreground transition-all duration-150"
                        id=[attr_id(&next_attrs)]
                        data-testid=[attr_testid(&next_attrs)]
                        data-on:click=[attr_on_click(&next_attrs)]
                    {
                        span class="pagination-label" { (shared::default_strings().pagination_next) }
                        (chevronRight())
                    }
                }
            } @else {
                span class="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-muted-foreground opacity-50" {
                    span class="pagination-label" { (shared::default_strings().pagination_next) }
                    (chevronRight())
                }
            }
        }
    }
}

#[must_use]
pub fn chevronLeft() -> Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" {
            path d="m15 18-6-6 6-6" {}
        }
    }
}

#[must_use]
pub fn chevronRight() -> Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" {
            path d="m9 18 6-6-6-6" {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_helpers_and_rendering_cover_states_and_attrs() {
        let base = PaginationItemProps::default();
        assert_eq!(base.page, 1);
        assert!(base.build_classes().contains("hover:bg-accent"));
        assert!(
            PaginationItemProps::default()
                .with_active(true)
                .build_classes()
                .contains("bg-primary text-primary-foreground")
        );
        assert!(
            PaginationItemProps::default()
                .with_disabled(true)
                .build_classes()
                .contains("cursor-not-allowed")
        );
        assert!(
            PaginationItemProps::default()
                .with_class("page-extra")
                .build_classes()
                .contains("page-extra")
        );
        let attrs = PaginationItemProps::default()
            .with_attribute("data-testid", "p1")
            .build_attributes();
        assert_eq!(attrs.get("data-testid").map(String::as_str), Some("p1"));

        let link = PaginationItem(
            &PaginationItemProps::default()
                .with_page(3)
                .with_href("/items?page=3")
                .with_active(true)
                .with_attribute("id", "page3"),
        )
        .into_string();
        assert!(link.starts_with(
            "<a href=\"/items?page=3\" class=\"inline-flex items-center justify-center"
        ));
        assert!(link.contains("aria-current=\"page\" id=\"page3\""));
        assert!(link.ends_with(">3</a>"));

        let disabled = PaginationItem(
            &PaginationItemProps::default()
                .with_page(4)
                .with_disabled(true)
                .with_attribute("data-testid", "disabled"),
        )
        .into_string();
        assert!(disabled.starts_with("<span class=\"inline-flex items-center justify-center"));
        assert!(disabled.contains("aria-disabled=\"true\" data-testid=\"disabled\""));
        assert!(disabled.ends_with(">4</span>"));
    }

    #[test]
    fn pagination_helpers_match_go_page_range_and_url_behaviour() {
        assert_eq!(buildPageHref("", 2), "?page=2");
        assert_eq!(
            buildPageHref("/en-us/quizzes?page={page}&status=ready", 3),
            "/en-us/quizzes?page=3&status=ready"
        );
        assert_eq!(
            buildPageHref("/en-us/quizzes?page=%7Bpage%7D&status=ready", 4),
            "/en-us/quizzes?page=4&status=ready"
        );
        assert_eq!(
            buildPageHref("/en-us/quizzes?page=%7bpage%7d", 5),
            "/en-us/quizzes?page=5"
        );
        assert_eq!(
            compactNavClasses(true),
            "inline-flex items-center gap-1.5 px-3 py-2 text-sm font-medium rounded-md border border-border min-h-[2.75rem] text-muted-foreground opacity-40 cursor-not-allowed pointer-events-none"
        );
        assert!(compactNavClasses(false).contains("hover:bg-accent"));
        assert!(fullNavClasses(true).contains("pointer-events-none"));
        assert!(fullNavClasses(false).contains("pagination-page"));

        let zero = PaginationProps {
            current_page: 0,
            total_pages: 0,
            max_visible: 0,
            show_first: false,
            show_last: false,
            show_prev_next: false,
            ..PaginationProps::default()
        }
        .normalize();
        assert_eq!(zero.current_page, 1);
        assert_eq!(zero.total_pages, 1);
        assert_eq!(zero.max_visible, 5);
        assert!(zero.show_first && zero.show_last && zero.show_prev_next);
        let toggles = PaginationProps::default()
            .with_show_first(false)
            .with_show_last(false)
            .with_show_prev_next(false);
        assert!(!toggles.show_first);
        assert!(!toggles.show_last);
        assert!(!toggles.show_prev_next);

        let all_fit = PaginationProps::default()
            .with_current_page(1)
            .with_total_pages(5)
            .with_max_visible(5);
        assert_eq!(all_fit.get_page_range(), vec![1, 2, 3, 4, 5]);
        assert_eq!(all_fit.first_visible_page(), 1);
        assert_eq!(all_fit.last_visible_page(), 5);
        assert!(!all_fit.should_render_first_page_link());
        assert!(!all_fit.should_render_last_page_link());

        let trimmed = PaginationProps::default()
            .with_current_page(5)
            .with_total_pages(10)
            .with_max_visible(5);
        assert_eq!(trimmed.get_page_range(), vec![3, 4, 5, 6, 7]);
        assert!(trimmed.contains_visible_page(5));
        assert!(trimmed.should_render_first_page_link());
        assert!(trimmed.should_render_last_page_link());
        assert!(trimmed.has_prev());
        assert!(trimmed.has_next());
        assert_eq!(
            PaginationProps::default()
                .with_current_page(1)
                .with_total_pages(10)
                .with_max_visible(5)
                .get_page_range(),
            vec![1, 2, 3, 4, 5]
        );
        assert_eq!(
            PaginationProps::default()
                .with_current_page(10)
                .with_total_pages(10)
                .with_max_visible(5)
                .get_page_range(),
            vec![6, 7, 8, 9, 10]
        );
        assert_eq!(
            trimmed
                .build_attributes()
                .get("data-smooth-preserve-scroll")
                .map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn pagination_renders_full_compact_responsive_list_and_hidden_modes() {
        let full = Pagination(
            &PaginationProps::default()
                .with_current_page(5)
                .with_total_pages(10)
                .with_base_href("/items?page={page}")
                .with_attribute("data-testid", "pager"),
        )
        .into_string();
        assert!(full.starts_with("<nav class=\"flex items-center gap-1\" aria-label=\"Pagination\" data-smooth-preserve-scroll=\"true\" data-testid=\"pager\">"));
        assert!(
            full.contains("href=\"/items?page=4\" class=\"inline-flex items-center justify-center")
        );
        assert!(full.contains("aria-label=\"Previous page\""));
        assert!(full.contains("href=\"/items?page=1\""));
        assert!(full.contains(">...</span>"));
        assert!(full.contains("href=\"/items?page=5\""));
        assert!(full.contains("aria-current=\"page\""));
        assert!(full.contains("href=\"/items?page=10\""));
        assert!(full.contains("href=\"/items?page=6\""));
        assert!(full.contains("aria-label=\"Next page\""));

        let compact = Pagination(
            &PaginationProps::default()
                .with_current_page(1)
                .with_total_pages(3)
                .with_base_href("/items?page={page}")
                .with_compact(true),
        )
        .into_string();
        assert!(compact.contains("<span class=\"inline-flex items-center gap-1.5"));
        assert!(compact.contains(
            "<span class=\"text-sm font-medium text-muted-foreground\">Page 1 of 3</span>"
        ));
        assert!(compact.contains("href=\"/items?page=2\""));

        let responsive = Pagination(
            &PaginationProps::default()
                .with_total_pages(2)
                .with_responsive(true),
        )
        .into_string();
        assert!(
            responsive.starts_with(
                "<nav aria-label=\"Pagination\" data-smooth-preserve-scroll=\"true\">"
            )
        );
        assert!(responsive.contains("pagination-full"));
        assert!(responsive.contains("pagination-compact"));

        let list = ListPaginationBar(
            &PaginationProps::default()
                .with_total_pages(2)
                .with_class("extra"),
        )
        .into_string();
        assert!(
            list.contains("class=\"flex items-center gap-1 justify-center lg:justify-end extra\"")
        );
        assert_eq!(
            ListPaginationBar(&PaginationProps::default()).into_string(),
            ""
        );

        let compact_last = paginationCompact(
            &PaginationProps::default()
                .with_current_page(3)
                .with_total_pages(3),
        )
        .into_string();
        assert!(compact_last.contains("href=\"?page=2\""));
        assert!(compact_last.contains("Next"));

        let full_last = paginationFull(
            &PaginationProps::default()
                .with_current_page(3)
                .with_total_pages(3),
        )
        .into_string();
        assert!(full_last.contains("<span class=\"inline-flex items-center justify-center"));

        assert_eq!(
            Pagination(&PaginationProps::default().with_total_pages(1)).into_string(),
            ""
        );
    }

    #[test]
    fn simple_pagination_renders_links_buttons_disabled_and_attrs() {
        let hidden = SimplePagination(&SimplePaginationProps::default()).into_string();
        assert_eq!(hidden, "");

        let links = SimplePagination(
            &SimplePaginationProps::default()
                .with_current_page(2)
                .with_total_pages(4)
                .with_prev_href("/items?page=1")
                .with_next_href("/items?page=3")
                .with_class("pager")
                .with_attribute("id", "simple")
                .with_prev_attribute("data-testid", "prev")
                .with_next_attribute("data-on:click", "$next()"),
        )
        .into_string();
        assert!(links.starts_with("<nav class=\"flex items-center justify-between gap-4 pager\" aria-label=\"Pagination\" data-smooth-preserve-scroll=\"true\" id=\"simple\">"));
        assert!(links.contains("<a href=\"/items?page=1\" class=\"inline-flex items-center gap-2"));
        assert!(links.contains("data-testid=\"prev\""));
        assert!(links.contains("<span class=\"pagination-label\">Previous</span>"));
        assert!(links.contains(
            "<span class=\"text-sm text-muted-foreground whitespace-nowrap\">Page 2 of 4</span>"
        ));
        assert!(links.contains("<a href=\"/items?page=3\" class=\"inline-flex items-center gap-2"));
        assert!(links.contains("data-on:click=\"$next()\""));
        assert!(links.contains("<span class=\"pagination-label\">Next</span>"));

        let buttons = SimplePagination(
            &SimplePaginationProps::default()
                .with_current_page(2)
                .with_total_pages(3),
        )
        .into_string();
        assert!(buttons.contains("<button type=\"button\" class=\"inline-flex items-center gap-2"));
        assert!(buttons.contains("<span class=\"pagination-label\">Previous</span>"));
        assert!(buttons.contains("<span class=\"pagination-label\">Next</span>"));

        let disabled = SimplePagination(
            &SimplePaginationProps::default()
                .with_current_page(1)
                .with_total_pages(2),
        )
        .into_string();
        assert!(disabled.contains("text-muted-foreground opacity-50"));
        assert!(disabled.contains("<button type=\"button\""));

        let next_disabled = SimplePagination(
            &SimplePaginationProps::default()
                .with_current_page(2)
                .with_total_pages(2),
        )
        .into_string();
        assert!(next_disabled.contains("<span class=\"pagination-label\">Next</span>"));
    }

    #[test]
    fn chevrons_render_expected_svg() {
        assert_eq!(
            chevronLeft().into_string(),
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"m15 18-6-6 6-6\"></path></svg>",
        );
        assert_eq!(
            chevronRight().into_string(),
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"m9 18 6-6-6-6\"></path></svg>",
        );
    }
}
