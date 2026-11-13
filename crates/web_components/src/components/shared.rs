use crate::components::FileSource;
use std::collections::BTreeMap;

pub const SIZE_XS: &str = "xs";
pub const SIZE_SM: &str = "sm";
pub const SIZE_MD: &str = "md";
pub const SIZE_LG: &str = "lg";
pub const SIZE_XL: &str = "xl";

pub const VARIANT_DEFAULT: &str = "default";
pub const VARIANT_PRIMARY: &str = "primary";
pub const VARIANT_SECONDARY: &str = "secondary";
pub const VARIANT_DESTRUCTIVE: &str = "destructive";
pub const VARIANT_SUCCESS: &str = "success";
pub const VARIANT_WARNING: &str = "warning";
pub const VARIANT_INFO: &str = "info";
pub const VARIANT_GHOST: &str = "ghost";
pub const VARIANT_OUTLINE: &str = "outline";
pub const VARIANT_LINK: &str = "link";

pub const AVATAR_SHAPE_CIRCLE: &str = "circle";
pub const AVATAR_SHAPE_SQUARE: &str = "square";
pub const AVATAR_SHAPE_ROUNDED: &str = "rounded";

pub const AVATAR_STATUS_NONE: &str = "none";
pub const AVATAR_STATUS_ONLINE: &str = "online";
pub const AVATAR_STATUS_OFFLINE: &str = "offline";
pub const AVATAR_STATUS_AWAY: &str = "away";
pub const AVATAR_STATUS_BUSY: &str = "busy";

pub const BADGE_STYLE_SOLID: &str = "solid";
pub const BADGE_STYLE_OUTLINE: &str = "outline";
pub const BADGE_STYLE_SOFT: &str = "soft";

pub const CHAT_ROLE_USER: &str = "user";
pub const CHAT_ROLE_ASSISTANT: &str = "assistant";
pub const CHAT_ROLE_SYSTEM: &str = "system";

pub const STATUS_BADGE_ACTIVE: &str = "active";
pub const STATUS_BADGE_INACTIVE: &str = "inactive";
pub const STATUS_BADGE_PENDING: &str = "pending";
pub const STATUS_BADGE_ERROR: &str = "error";
pub const STATUS_BADGE_SUCCESS: &str = "success";
pub const STATUS_BADGE_WARNING: &str = "warning";

pub const NEOMORPH_FLAT: &str = "flat";
pub const NEOMORPH_FLAT_SM: &str = "flat-sm";
pub const NEOMORPH_FLAT_LG: &str = "flat-lg";
pub const NEOMORPH_CONCAVE: &str = "concave";
pub const NEOMORPH_CONCAVE_SM: &str = "concave-sm";
pub const NEOMORPH_CONVEX: &str = "convex";
pub const NEOMORPH_GLASS: &str = "glass";

pub const POSITION_TOP: &str = "top";
pub const POSITION_BOTTOM: &str = "bottom";
pub const POSITION_LEFT: &str = "left";
pub const POSITION_RIGHT: &str = "right";
pub const POSITION_TOP_LEFT: &str = "top-left";
pub const POSITION_TOP_RIGHT: &str = "top-right";
pub const POSITION_BOTTOM_LEFT: &str = "bottom-left";
pub const POSITION_BOTTOM_RIGHT: &str = "bottom-right";
pub const POSITION_TOP_CENTER: &str = "top-center";
pub const POSITION_BOTTOM_CENTER: &str = "bottom-center";

pub const ORIENTATION_HORIZONTAL: &str = "horizontal";
pub const ORIENTATION_VERTICAL: &str = "vertical";

pub const ALIGN_START: &str = "start";
pub const ALIGN_CENTER: &str = "center";
pub const ALIGN_END: &str = "end";

pub const LOADING_STATE_IDLE: &str = "idle";
pub const LOADING_STATE_LOADING: &str = "loading";
pub const LOADING_STATE_SUCCESS: &str = "success";
pub const LOADING_STATE_ERROR: &str = "error";

pub const INPUT_TYPE_TEXT: &str = "text";
pub const INPUT_TYPE_EMAIL: &str = "email";
pub const INPUT_TYPE_PASSWORD: &str = "password";
pub const INPUT_TYPE_NUMBER: &str = "number";
pub const INPUT_TYPE_TEL: &str = "tel";
pub const INPUT_TYPE_URL: &str = "url";
pub const INPUT_TYPE_SEARCH: &str = "search";
pub const INPUT_TYPE_DATE: &str = "date";
pub const INPUT_TYPE_TIME: &str = "time";
pub const INPUT_TYPE_DATE_TIME_LOCAL: &str = "datetime-local";
pub const INPUT_TYPE_HIDDEN: &str = "hidden";

pub const SKELETON_BASIC: &str = "basic";
pub const SKELETON_CARD: &str = "card";
pub const SKELETON_LIST: &str = "list";
pub const SKELETON_TABLE: &str = "table";
pub const SKELETON_AVATAR: &str = "avatar";
pub const SKELETON_TEXT: &str = "text";

pub const HEADING_H1: &str = "h1";
pub const HEADING_H2: &str = "h2";
pub const HEADING_H3: &str = "h3";
pub const HEADING_H4: &str = "h4";
pub const HEADING_H5: &str = "h5";
pub const HEADING_H6: &str = "h6";

pub const CONTAINER_SM: &str = "sm";
pub const CONTAINER_MD: &str = "md";
pub const CONTAINER_LG: &str = "lg";
pub const CONTAINER_XL: &str = "xl";
pub const CONTAINER_FULL: &str = "full";

pub const TEXT_WEIGHT_NORMAL: &str = "normal";
pub const TEXT_WEIGHT_MEDIUM: &str = "medium";
pub const TEXT_WEIGHT_SEMIBOLD: &str = "semibold";
pub const TEXT_WEIGHT_BOLD: &str = "bold";

pub const TEXT_COLOR_DEFAULT: &str = "default";
pub const TEXT_COLOR_MUTED: &str = "muted";
pub const TEXT_COLOR_PRIMARY: &str = "primary";
pub const TEXT_COLOR_SECONDARY: &str = "secondary";
pub const TEXT_COLOR_SUCCESS: &str = "success";
pub const TEXT_COLOR_WARNING: &str = "warning";
pub const TEXT_COLOR_ERROR: &str = "error";

pub const DIVIDER_SOLID: &str = "solid";
pub const DIVIDER_DASHED: &str = "dashed";
pub const DIVIDER_DOTTED: &str = "dotted";

pub const CAROUSEL_PREV: &str = "prev";
pub const CAROUSEL_NEXT: &str = "next";

pub const ASPECT_RATIO_16X9: &str = "16/9";
pub const ASPECT_RATIO_4X3: &str = "4/3";
pub const ASPECT_RATIO_1X1: &str = "1/1";
pub const ASPECT_RATIO_9X16: &str = "9/16";

pub const MENU_ALIGN_START: &str = "start";
pub const MENU_ALIGN_CENTER: &str = "center";
pub const MENU_ALIGN_END: &str = "end";

pub const MENU_SIDE_TOP: &str = "top";
pub const MENU_SIDE_RIGHT: &str = "right";
pub const MENU_SIDE_BOTTOM: &str = "bottom";
pub const MENU_SIDE_LEFT: &str = "left";

pub const CARD_FOOTER_START: &str = "start";
pub const CARD_FOOTER_CENTER: &str = "center";
pub const CARD_FOOTER_END: &str = "end";
pub const CARD_FOOTER_BETWEEN: &str = "between";

pub const STACK_HORIZONTAL: &str = "horizontal";
pub const STACK_VERTICAL: &str = "vertical";

pub const STACK_ALIGN_START: &str = "start";
pub const STACK_ALIGN_CENTER: &str = "center";
pub const STACK_ALIGN_END: &str = "end";
pub const STACK_ALIGN_STRETCH: &str = "stretch";

pub const STACK_JUSTIFY_START: &str = "start";
pub const STACK_JUSTIFY_CENTER: &str = "center";
pub const STACK_JUSTIFY_END: &str = "end";
pub const STACK_JUSTIFY_BETWEEN: &str = "between";
pub const STACK_JUSTIFY_AROUND: &str = "around";

pub const ELEMENT_DIV: &str = "div";
pub const ELEMENT_SPAN: &str = "span";
pub const ELEMENT_A: &str = "a";
pub const ELEMENT_BUTTON: &str = "button";
pub const ELEMENT_ARTICLE: &str = "article";
pub const ELEMENT_SECTION: &str = "section";
pub const ELEMENT_ASIDE: &str = "aside";

pub const TYPES_SOURCE: &str = r#"type Size string"#;
pub const VARIANTS_SOURCE: &str = r#"VariantPrimary"#;
pub const STRINGS_SOURCE: &str = r#"DefaultStrings"#;

#[must_use]
pub fn source_files() -> Vec<FileSource> {
    vec![
        FileSource {
            family: "shared",
            module: "shared",
            path: "library/components/shared/types.go",
            source: TYPES_SOURCE,
        },
        FileSource {
            family: "shared",
            module: "shared",
            path: "library/components/shared/variants.go",
            source: VARIANTS_SOURCE,
        },
        FileSource {
            family: "shared",
            module: "shared",
            path: "library/components/shared/strings.go",
            source: STRINGS_SOURCE,
        },
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComponentStrings {
    pub pagination_aria_label: &'static str,
    pub pagination_previous: &'static str,
    pub pagination_next: &'static str,
    pub pagination_first: &'static str,
    pub pagination_last: &'static str,
    pub pagination_previous_page: &'static str,
    pub pagination_next_page: &'static str,
    pub pagination_page_of_total: &'static str,
    pub pagination_go_to_slide: &'static str,
    pub loading_aria_label: &'static str,
    pub loading_text: &'static str,
    pub empty_state_no_data: &'static str,
    pub empty_state_no_results: &'static str,
    pub empty_state_no_results_query: &'static str,
    pub dialog_close_aria_label: &'static str,
    pub badge_remove_aria_label: &'static str,
    pub input_required_aria_label: &'static str,
    pub input_required_symbol: &'static str,
    pub input_optional_text: &'static str,
    pub input_email_label: &'static str,
    pub input_password_label: &'static str,
    pub input_url_prefix: &'static str,
    pub skeleton_loading_card: &'static str,
    pub skeleton_loading_table: &'static str,
    pub skeleton_loading_list: &'static str,
    pub skeleton_loading_stats: &'static str,
    pub skeleton_loading: &'static str,
    pub code_copy_aria_label: &'static str,
    pub code_copy_text: &'static str,
    pub chat_typing_aria_label: &'static str,
    pub chat_user_avatar: &'static str,
    pub chat_ai_avatar: &'static str,
    pub image_view_full_size: &'static str,
    pub play_video_aria_label: &'static str,
    pub bottom_nav_aria_label: &'static str,
    pub progress_loading_aria_label: &'static str,
    pub carousel_prev_slide: &'static str,
    pub carousel_next_slide: &'static str,
    pub nav_arrow_previous: &'static str,
    pub nav_arrow_next: &'static str,
    pub nav_arrow_up: &'static str,
    pub nav_arrow_down: &'static str,
    pub ai_icon_aria_label: &'static str,
}

#[must_use]
pub fn default_strings() -> ComponentStrings {
    ComponentStrings {
        pagination_aria_label: "Pagination",
        pagination_previous: "Previous",
        pagination_next: "Next",
        pagination_first: "First",
        pagination_last: "Last",
        pagination_previous_page: "Previous page",
        pagination_next_page: "Next page",
        pagination_page_of_total: "Page %d of %d",
        pagination_go_to_slide: "Go to slide %d",
        loading_aria_label: "Loading",
        loading_text: "Loading...",
        empty_state_no_data: "No Data",
        empty_state_no_results: "No Results Found",
        empty_state_no_results_query: "No results found for \"%s\". Try adjusting your search.",
        dialog_close_aria_label: "Close dialog",
        badge_remove_aria_label: "Remove",
        input_required_aria_label: "required",
        input_required_symbol: "*",
        input_optional_text: "(optional)",
        input_email_label: "Email",
        input_password_label: "Password",
        input_url_prefix: "https://",
        skeleton_loading_card: "Loading card",
        skeleton_loading_table: "Loading table",
        skeleton_loading_list: "Loading list",
        skeleton_loading_stats: "Loading statistics",
        skeleton_loading: "Loading",
        code_copy_aria_label: "Copy code",
        code_copy_text: "Copy",
        chat_typing_aria_label: "Typing...",
        chat_user_avatar: "User avatar",
        chat_ai_avatar: "AI avatar",
        image_view_full_size: "View full size",
        play_video_aria_label: "Play video",
        bottom_nav_aria_label: "Bottom navigation",
        progress_loading_aria_label: "Loading",
        carousel_prev_slide: "Previous slide",
        carousel_next_slide: "Next slide",
        nav_arrow_previous: "Previous",
        nav_arrow_next: "Next",
        nav_arrow_up: "Up",
        nav_arrow_down: "Down",
        ai_icon_aria_label: "AI",
    }
}

#[must_use]
pub fn build_classes(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[must_use]
pub fn attr_value<'a>(attrs: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    attrs
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

#[must_use]
pub fn optional_attr(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

#[must_use]
pub fn neomorph_variants() -> &'static [(&'static str, &'static str)] {
    &[
        ("flat", "neomorph-flat"),
        ("flat-sm", "neomorph-flat-sm"),
        ("flat-lg", "neomorph-flat-lg"),
        ("concave", "neomorph-concave"),
        ("concave-sm", "neomorph-concave-sm"),
        ("convex", "neomorph-convex"),
        ("glass", "neomorph-glass"),
    ]
}

#[must_use]
pub fn button_variant_classes(variant: &str) -> &'static str {
    match variant {
        VARIANT_PRIMARY => "interactive-btn btn-primary-gradient text-primary-foreground",
        VARIANT_SECONDARY => {
            "bg-secondary text-secondary-foreground hover:bg-secondary/80 hover:-translate-y-px active:translate-y-0 active:scale-[0.98]"
        }
        VARIANT_DESTRUCTIVE => {
            "interactive-btn interactive-btn-destructive bg-destructive text-destructive-foreground"
        }
        VARIANT_SUCCESS => {
            "interactive-btn interactive-btn-success bg-success text-success-foreground"
        }
        VARIANT_WARNING => {
            "interactive-btn interactive-btn-warning bg-warning text-warning-foreground"
        }
        VARIANT_INFO => "interactive-btn interactive-btn-info bg-info text-info-foreground",
        VARIANT_GHOST => "text-foreground hover:bg-muted hover:text-foreground active:scale-[0.98]",
        VARIANT_OUTLINE => {
            "border border-border text-foreground bg-transparent hover:bg-muted hover:border-primary hover:text-primary hover:-translate-y-px active:translate-y-0 active:scale-[0.98]"
        }
        VARIANT_LINK => "text-primary btn-link-underline",
        _ => "interactive-btn btn-primary-gradient text-primary-foreground",
    }
}

#[must_use]
pub fn button_size_classes(size: &str) -> &'static str {
    match size {
        SIZE_XS => "h-8 px-2 text-xs rounded-md",
        SIZE_SM => "h-9 px-3 text-sm rounded-md",
        SIZE_LG => "h-12 px-6 text-lg rounded-lg",
        SIZE_XL => "h-14 px-8 text-xl rounded-xl",
        _ => "h-10 px-4 text-base rounded-lg",
    }
}

#[must_use]
pub fn input_size_classes(size: &str) -> &'static str {
    match size {
        SIZE_SM => "h-8 px-3 py-1 text-xs rounded-md",
        SIZE_LG => "h-12 px-4 py-3 text-base rounded-lg",
        _ => "h-10 px-3 py-2 text-sm rounded-md",
    }
}

#[must_use]
pub fn textarea_size_classes(size: &str) -> &'static str {
    match size {
        SIZE_SM => "h-auto px-3 py-2 text-sm rounded-md",
        SIZE_LG => "h-auto px-4 py-3 text-base rounded-lg",
        _ => "h-auto px-3 py-3 text-base rounded-md",
    }
}

#[must_use]
pub fn badge_size_classes(size: &str) -> &'static str {
    match size {
        SIZE_SM => "px-2 py-0.5 text-xs",
        SIZE_LG => "px-4 py-1.5 text-base",
        _ => "px-3 py-1 text-sm",
    }
}

#[must_use]
pub fn badge_variant_classes(variant: &str) -> &'static str {
    match variant {
        VARIANT_PRIMARY => "bg-primary text-primary-foreground",
        VARIANT_SECONDARY => "bg-secondary text-secondary-foreground",
        VARIANT_DESTRUCTIVE => "bg-destructive text-destructive-foreground",
        VARIANT_SUCCESS => "bg-success text-success-foreground",
        VARIANT_WARNING => "bg-warning text-warning-foreground",
        VARIANT_INFO => "bg-info text-info-foreground",
        VARIANT_OUTLINE => "border-2 border-border bg-transparent text-foreground",
        _ => "bg-muted text-muted-foreground",
    }
}

#[must_use]
pub fn avatar_size_classes(size: &str) -> &'static str {
    match size {
        SIZE_XS => "h-6 w-6 text-xs",
        SIZE_SM => "h-8 w-8 text-sm",
        SIZE_LG => "h-12 w-12 text-lg",
        SIZE_XL => "h-16 w-16 text-xl",
        _ => "h-10 w-10 text-base",
    }
}

#[must_use]
pub fn progress_size_classes(size: &str) -> &'static str {
    match size {
        SIZE_SM => "h-1",
        SIZE_LG => "h-4",
        _ => "h-2",
    }
}

#[must_use]
pub fn progress_variant_classes(variant: &str) -> &'static str {
    match variant {
        VARIANT_PRIMARY => "bg-primary",
        VARIANT_SECONDARY => "bg-secondary",
        VARIANT_DESTRUCTIVE => "bg-destructive",
        VARIANT_SUCCESS => "bg-success",
        VARIANT_WARNING => "bg-warning",
        VARIANT_INFO => "bg-info",
        _ => "bg-primary",
    }
}

#[must_use]
pub fn alert_variant_classes(variant: &str) -> &'static str {
    match variant {
        VARIANT_INFO => "border border-info bg-info-muted text-info-foreground shadow-sm",
        VARIANT_SUCCESS => {
            "border border-success bg-success-muted text-success-foreground shadow-sm"
        }
        VARIANT_WARNING => {
            "border border-warning bg-warning-muted text-warning-foreground shadow-sm"
        }
        VARIANT_DESTRUCTIVE => {
            "border border-destructive bg-destructive-muted text-destructive-foreground shadow-sm"
        }
        _ => "border border-border border-l-4 border-l-border bg-card text-foreground",
    }
}

#[must_use]
pub fn alert_icon_color_class(variant: &str) -> &'static str {
    match variant {
        VARIANT_INFO => "text-info",
        VARIANT_SUCCESS => "text-success",
        VARIANT_WARNING => "text-warning",
        VARIANT_DESTRUCTIVE => "text-destructive",
        _ => "text-muted-foreground",
    }
}

#[must_use]
pub fn toast_position_classes(position: &str) -> &'static str {
    match position {
        POSITION_TOP_LEFT => "top-4 left-4",
        POSITION_TOP_RIGHT => "top-4 right-4",
        POSITION_BOTTOM_LEFT => "bottom-4 left-4",
        POSITION_BOTTOM_RIGHT => "bottom-4 right-4",
        POSITION_TOP_CENTER => "top-4 left-1/2 -translate-x-1/2",
        POSITION_BOTTOM_CENTER => "bottom-4 left-1/2 -translate-x-1/2",
        _ => "top-4 right-4",
    }
}

#[must_use]
pub fn tooltip_position_classes(position: &str) -> &'static str {
    match position {
        POSITION_TOP => "bottom-full left-1/2 -translate-x-1/2 mb-2",
        POSITION_RIGHT => "left-full top-1/2 -translate-y-1/2 ml-2",
        POSITION_BOTTOM => "top-full left-1/2 -translate-x-1/2 mt-2",
        POSITION_LEFT => "right-full top-1/2 -translate-y-1/2 mr-2",
        _ => "bottom-full left-1/2 -translate-x-1/2 mb-2",
    }
}

#[must_use]
pub fn int_to_grid_string(value: i32) -> &'static str {
    match value {
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        6 => "6",
        _ => "3",
    }
}

#[must_use]
pub fn avatar_shape_classes(shape: &str) -> &'static str {
    match shape {
        AVATAR_SHAPE_SQUARE => "rounded-none",
        AVATAR_SHAPE_ROUNDED => "rounded-lg",
        _ => "rounded-full",
    }
}

#[must_use]
pub fn avatar_status_classes(status: &str) -> &'static str {
    match status {
        AVATAR_STATUS_ONLINE => "bg-success",
        AVATAR_STATUS_OFFLINE => "bg-muted-foreground",
        AVATAR_STATUS_AWAY => "bg-warning",
        AVATAR_STATUS_BUSY => "bg-destructive",
        _ => "",
    }
}

#[must_use]
pub fn badge_style_classes(style: &str, variant: &str) -> &'static str {
    match style {
        BADGE_STYLE_OUTLINE => badge_outline_variant_classes(variant),
        BADGE_STYLE_SOFT => badge_soft_variant_classes(variant),
        _ => badge_variant_classes(variant),
    }
}

#[must_use]
pub fn badge_outline_variant_classes(variant: &str) -> &'static str {
    match variant {
        VARIANT_PRIMARY => "border-2 border-primary text-primary bg-transparent",
        VARIANT_SECONDARY => "border-2 border-secondary text-secondary bg-transparent",
        VARIANT_DESTRUCTIVE => "border-2 border-destructive text-destructive bg-transparent",
        VARIANT_SUCCESS => "border-2 border-success text-success bg-transparent",
        VARIANT_WARNING => "border-2 border-warning text-warning bg-transparent",
        VARIANT_INFO => "border-2 border-info text-info bg-transparent",
        _ => "border-2 border-border text-foreground bg-transparent",
    }
}

#[must_use]
pub fn badge_soft_variant_classes(variant: &str) -> &'static str {
    match variant {
        VARIANT_PRIMARY => "bg-primary/15 text-primary",
        VARIANT_SECONDARY => "bg-secondary text-secondary-foreground",
        VARIANT_DESTRUCTIVE => "bg-destructive-muted text-destructive",
        VARIANT_SUCCESS => "bg-success-muted text-success",
        VARIANT_WARNING => "bg-warning-muted text-warning",
        VARIANT_INFO => "bg-info-muted text-info",
        _ => "bg-muted text-muted-foreground",
    }
}

#[must_use]
pub fn chat_role_classes(role: &str) -> &'static str {
    match role {
        CHAT_ROLE_USER => "bg-primary text-primary-foreground ml-auto",
        CHAT_ROLE_ASSISTANT => "bg-muted text-muted-foreground mr-auto",
        CHAT_ROLE_SYSTEM => "bg-secondary/50 text-secondary-foreground mx-auto text-center italic",
        _ => "bg-muted text-muted-foreground",
    }
}

#[must_use]
pub fn status_badge_classes(status: &str) -> &'static str {
    match status {
        STATUS_BADGE_ACTIVE | STATUS_BADGE_SUCCESS => "bg-success-muted text-success",
        STATUS_BADGE_INACTIVE => "bg-muted text-muted-foreground",
        STATUS_BADGE_PENDING | STATUS_BADGE_WARNING => "bg-warning-muted text-warning",
        STATUS_BADGE_ERROR => "bg-destructive-muted text-destructive",
        _ => "bg-muted text-muted-foreground",
    }
}

#[must_use]
pub fn container_size_classes(size: &str) -> &'static str {
    match size {
        CONTAINER_SM => "max-w-sm",
        CONTAINER_LG => "max-w-4xl",
        CONTAINER_XL => "max-w-6xl",
        CONTAINER_FULL => "max-w-full",
        _ => "max-w-2xl",
    }
}

#[must_use]
pub fn text_weight_classes(weight: &str) -> &'static str {
    match weight {
        TEXT_WEIGHT_MEDIUM => "font-medium",
        TEXT_WEIGHT_SEMIBOLD => "font-semibold",
        TEXT_WEIGHT_BOLD => "font-bold",
        _ => "font-normal",
    }
}

#[must_use]
pub fn text_color_classes(color: &str) -> &'static str {
    match color {
        TEXT_COLOR_MUTED => "text-muted-foreground",
        TEXT_COLOR_PRIMARY => "text-primary",
        TEXT_COLOR_SECONDARY => "text-secondary",
        TEXT_COLOR_SUCCESS => "text-success",
        TEXT_COLOR_WARNING => "text-warning",
        TEXT_COLOR_ERROR => "text-destructive",
        _ => "text-foreground",
    }
}

#[must_use]
pub fn text_size_classes(size: &str) -> &'static str {
    match size {
        SIZE_XS => "text-xs",
        SIZE_SM => "text-sm",
        SIZE_LG => "text-lg",
        SIZE_XL => "text-xl",
        _ => "text-base",
    }
}

#[must_use]
pub fn divider_variant_classes(variant: &str) -> &'static str {
    match variant {
        DIVIDER_DASHED => "border-dashed",
        DIVIDER_DOTTED => "border-dotted",
        _ => "border-solid",
    }
}

#[must_use]
pub fn aspect_ratio_classes(ratio: &str) -> &'static str {
    match ratio {
        ASPECT_RATIO_4X3 => "aspect-[4/3]",
        ASPECT_RATIO_1X1 => "aspect-square",
        ASPECT_RATIO_9X16 => "aspect-[9/16]",
        _ => "aspect-video",
    }
}

#[must_use]
pub fn stack_direction_classes(direction: &str) -> &'static str {
    match direction {
        STACK_HORIZONTAL => "flex-row",
        _ => "flex-col",
    }
}

#[must_use]
pub fn stack_align_classes(align: &str) -> &'static str {
    match align {
        STACK_ALIGN_START => "items-start",
        STACK_ALIGN_END => "items-end",
        STACK_ALIGN_STRETCH => "items-stretch",
        _ => "items-center",
    }
}

#[must_use]
pub fn stack_justify_classes(justify: &str) -> &'static str {
    match justify {
        STACK_JUSTIFY_START => "justify-start",
        STACK_JUSTIFY_END => "justify-end",
        STACK_JUSTIFY_BETWEEN => "justify-between",
        STACK_JUSTIFY_AROUND => "justify-around",
        _ => "justify-center",
    }
}

#[must_use]
pub fn gap_classes(gap: i32) -> &'static str {
    match gap {
        0 => "gap-0",
        1 => "gap-1",
        2 => "gap-2",
        3 => "gap-3",
        4 => "gap-4",
        5 => "gap-5",
        6 => "gap-6",
        8 => "gap-8",
        10 => "gap-10",
        12 => "gap-12",
        _ => "gap-4",
    }
}

#[must_use]
pub fn heading_level_classes(level: &str) -> &'static str {
    match level {
        HEADING_H1 => "text-4xl font-bold tracking-tight",
        HEADING_H2 => "text-3xl font-semibold tracking-tight",
        HEADING_H3 => "text-2xl font-semibold",
        HEADING_H4 => "text-xl font-semibold",
        HEADING_H5 => "text-lg font-medium",
        HEADING_H6 => "text-base font-medium",
        _ => "text-2xl font-semibold",
    }
}

#[must_use]
pub fn skeleton_variant_classes(variant: &str) -> &'static str {
    match variant {
        SKELETON_CARD => "rounded-lg",
        SKELETON_LIST => "rounded-md",
        SKELETON_TABLE => "rounded-sm",
        SKELETON_AVATAR => "rounded-full",
        SKELETON_TEXT => "rounded-sm h-4",
        _ => "rounded-md",
    }
}

#[must_use]
pub fn card_footer_align_classes(align: &str) -> &'static str {
    match align {
        CARD_FOOTER_START => "justify-start",
        CARD_FOOTER_END => "justify-end",
        CARD_FOOTER_BETWEEN => "justify-between",
        _ => "justify-center",
    }
}

#[must_use]
pub fn switch_size_classes(size: &str) -> &'static str {
    match size {
        SIZE_XS => "h-4 w-7",
        SIZE_SM => "h-5 w-9",
        SIZE_LG => "h-7 w-[52px]",
        SIZE_XL => "h-8 w-[60px]",
        _ => "h-6 w-11",
    }
}

#[must_use]
pub fn switch_thumb_size_classes(size: &str) -> &'static str {
    match size {
        SIZE_XS => "h-3 w-3 data-[state=checked]:translate-x-3",
        SIZE_SM => "h-4 w-4 data-[state=checked]:translate-x-4",
        SIZE_LG => "h-6 w-6 data-[state=checked]:translate-x-[25px]",
        SIZE_XL => "h-7 w-7 data-[state=checked]:translate-x-[29px]",
        _ => "h-5 w-5 data-[state=checked]:translate-x-5",
    }
}

#[must_use]
pub fn tooltip_animation_classes(position: &str) -> &'static str {
    match position {
        POSITION_TOP => "animate-tooltip-in-top",
        POSITION_BOTTOM => "animate-tooltip-in-bottom",
        POSITION_LEFT => "animate-tooltip-in-left",
        POSITION_RIGHT => "animate-tooltip-in-right",
        _ => "animate-tooltip-in-top",
    }
}

#[must_use]
pub fn toast_slide_in_classes(position: &str) -> &'static str {
    match position {
        POSITION_TOP_LEFT | POSITION_BOTTOM_LEFT => "animate-toast-in-left",
        POSITION_TOP_RIGHT | POSITION_BOTTOM_RIGHT => "animate-toast-in-right",
        POSITION_TOP_CENTER => "animate-toast-in-top",
        POSITION_BOTTOM_CENTER => "animate-toast-in-bottom",
        _ => "animate-toast-in-right",
    }
}

#[must_use]
pub fn toast_accent_classes(variant: &str) -> &'static str {
    match variant {
        VARIANT_INFO => "toast-accent-info",
        VARIANT_SUCCESS => "toast-accent-success",
        VARIANT_WARNING => "toast-accent-warning",
        VARIANT_DESTRUCTIVE => "toast-accent-destructive",
        _ => "toast-accent-default",
    }
}

#[must_use]
pub fn toast_progress_classes(variant: &str) -> &'static str {
    match variant {
        VARIANT_INFO => "toast-progress-bar toast-progress-info",
        VARIANT_SUCCESS => "toast-progress-bar toast-progress-success",
        VARIANT_WARNING => "toast-progress-bar toast-progress-warning",
        VARIANT_DESTRUCTIVE => "toast-progress-bar toast-progress-destructive",
        _ => "toast-progress-bar toast-progress-default",
    }
}

#[must_use]
pub fn menu_side_classes(side: &str) -> &'static str {
    match side {
        MENU_SIDE_TOP => "bottom-full mb-2",
        MENU_SIDE_RIGHT => "left-full ml-2",
        MENU_SIDE_LEFT => "right-full mr-2",
        _ => "top-full mt-2",
    }
}

#[must_use]
pub fn menu_align_classes(align: &str) -> &'static str {
    match align {
        MENU_ALIGN_END => "right-0",
        MENU_ALIGN_CENTER => "left-1/2 -translate-x-1/2",
        _ => "left-0",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_helpers_match_go_defaults() {
        assert_eq!(
            button_variant_classes(VARIANT_PRIMARY),
            "interactive-btn btn-primary-gradient text-primary-foreground"
        );
        assert_eq!(
            button_size_classes(SIZE_MD),
            "h-10 px-4 text-base rounded-lg"
        );
        assert_eq!(
            badge_style_classes(BADGE_STYLE_SOFT, VARIANT_SUCCESS),
            "bg-success-muted text-success"
        );
        assert_eq!(
            toast_position_classes(POSITION_BOTTOM_CENTER),
            "bottom-4 left-1/2 -translate-x-1/2"
        );
        assert_eq!(optional_attr(""), None);
        assert_eq!(optional_attr("value"), Some("value"));
    }

    #[test]
    fn default_component_strings_match_go_source() {
        let strings = default_strings();

        assert_eq!(strings.pagination_aria_label, "Pagination");
        assert_eq!(strings.loading_text, "Loading...");
        assert_eq!(strings.input_url_prefix, "https://");
        assert_eq!(strings.ai_icon_aria_label, "AI");
    }

    #[test]
    fn source_files_and_shared_class_helpers_cover_every_branch() {
        let sources = source_files();
        assert_eq!(sources.len(), 3);
        assert_eq!(sources[0].path, "library/components/shared/types.go");
        assert!(sources[0].source.contains("type Size string"));
        assert!(sources[1].source.contains("VariantPrimary"));
        assert!(sources[2].source.contains("DefaultStrings"));

        assert_eq!(
            neomorph_variants(),
            &[
                ("flat", "neomorph-flat"),
                ("flat-sm", "neomorph-flat-sm"),
                ("flat-lg", "neomorph-flat-lg"),
                ("concave", "neomorph-concave"),
                ("concave-sm", "neomorph-concave-sm"),
                ("convex", "neomorph-convex"),
                ("glass", "neomorph-glass"),
            ]
        );

        for (variant, expected) in [
            (
                VARIANT_PRIMARY,
                "interactive-btn btn-primary-gradient text-primary-foreground",
            ),
            (
                VARIANT_SECONDARY,
                "bg-secondary text-secondary-foreground hover:bg-secondary/80 hover:-translate-y-px active:translate-y-0 active:scale-[0.98]",
            ),
            (
                VARIANT_DESTRUCTIVE,
                "interactive-btn interactive-btn-destructive bg-destructive text-destructive-foreground",
            ),
            (
                VARIANT_SUCCESS,
                "interactive-btn interactive-btn-success bg-success text-success-foreground",
            ),
            (
                VARIANT_WARNING,
                "interactive-btn interactive-btn-warning bg-warning text-warning-foreground",
            ),
            (
                VARIANT_INFO,
                "interactive-btn interactive-btn-info bg-info text-info-foreground",
            ),
            (
                VARIANT_GHOST,
                "text-foreground hover:bg-muted hover:text-foreground active:scale-[0.98]",
            ),
            (
                VARIANT_OUTLINE,
                "border border-border text-foreground bg-transparent hover:bg-muted hover:border-primary hover:text-primary hover:-translate-y-px active:translate-y-0 active:scale-[0.98]",
            ),
            (VARIANT_LINK, "text-primary btn-link-underline"),
            (
                "other",
                "interactive-btn btn-primary-gradient text-primary-foreground",
            ),
        ] {
            assert_eq!(button_variant_classes(variant), expected);
        }

        for (size, expected) in [
            (SIZE_XS, "h-8 px-2 text-xs rounded-md"),
            (SIZE_SM, "h-9 px-3 text-sm rounded-md"),
            (SIZE_LG, "h-12 px-6 text-lg rounded-lg"),
            (SIZE_XL, "h-14 px-8 text-xl rounded-xl"),
            ("other", "h-10 px-4 text-base rounded-lg"),
        ] {
            assert_eq!(button_size_classes(size), expected);
        }

        for (size, expected) in [
            (SIZE_SM, "h-8 px-3 py-1 text-xs rounded-md"),
            (SIZE_LG, "h-12 px-4 py-3 text-base rounded-lg"),
            ("other", "h-10 px-3 py-2 text-sm rounded-md"),
        ] {
            assert_eq!(input_size_classes(size), expected);
        }

        for (size, expected) in [
            (SIZE_SM, "h-auto px-3 py-2 text-sm rounded-md"),
            (SIZE_LG, "h-auto px-4 py-3 text-base rounded-lg"),
            ("other", "h-auto px-3 py-3 text-base rounded-md"),
        ] {
            assert_eq!(textarea_size_classes(size), expected);
        }

        for (size, expected) in [
            (SIZE_SM, "px-2 py-0.5 text-xs"),
            (SIZE_LG, "px-4 py-1.5 text-base"),
            ("other", "px-3 py-1 text-sm"),
        ] {
            assert_eq!(badge_size_classes(size), expected);
        }

        for (variant, expected) in [
            (VARIANT_PRIMARY, "bg-primary text-primary-foreground"),
            (VARIANT_SECONDARY, "bg-secondary text-secondary-foreground"),
            (
                VARIANT_DESTRUCTIVE,
                "bg-destructive text-destructive-foreground",
            ),
            (VARIANT_SUCCESS, "bg-success text-success-foreground"),
            (VARIANT_WARNING, "bg-warning text-warning-foreground"),
            (VARIANT_INFO, "bg-info text-info-foreground"),
            (
                VARIANT_OUTLINE,
                "border-2 border-border bg-transparent text-foreground",
            ),
            ("other", "bg-muted text-muted-foreground"),
        ] {
            assert_eq!(badge_variant_classes(variant), expected);
        }

        for (size, expected) in [
            (SIZE_XS, "h-6 w-6 text-xs"),
            (SIZE_SM, "h-8 w-8 text-sm"),
            (SIZE_LG, "h-12 w-12 text-lg"),
            (SIZE_XL, "h-16 w-16 text-xl"),
            ("other", "h-10 w-10 text-base"),
        ] {
            assert_eq!(avatar_size_classes(size), expected);
        }

        for (size, expected) in [(SIZE_SM, "h-1"), (SIZE_LG, "h-4"), ("other", "h-2")] {
            assert_eq!(progress_size_classes(size), expected);
        }

        for (variant, expected) in [
            (VARIANT_PRIMARY, "bg-primary"),
            (VARIANT_SECONDARY, "bg-secondary"),
            (VARIANT_DESTRUCTIVE, "bg-destructive"),
            (VARIANT_SUCCESS, "bg-success"),
            (VARIANT_WARNING, "bg-warning"),
            (VARIANT_INFO, "bg-info"),
            ("other", "bg-primary"),
        ] {
            assert_eq!(progress_variant_classes(variant), expected);
        }

        for (variant, expected) in [
            (
                VARIANT_INFO,
                "border border-info bg-info-muted text-info-foreground shadow-sm",
            ),
            (
                VARIANT_SUCCESS,
                "border border-success bg-success-muted text-success-foreground shadow-sm",
            ),
            (
                VARIANT_WARNING,
                "border border-warning bg-warning-muted text-warning-foreground shadow-sm",
            ),
            (
                VARIANT_DESTRUCTIVE,
                "border border-destructive bg-destructive-muted text-destructive-foreground shadow-sm",
            ),
            (
                "other",
                "border border-border border-l-4 border-l-border bg-card text-foreground",
            ),
        ] {
            assert_eq!(alert_variant_classes(variant), expected);
        }

        for (variant, expected) in [
            (VARIANT_INFO, "text-info"),
            (VARIANT_SUCCESS, "text-success"),
            (VARIANT_WARNING, "text-warning"),
            (VARIANT_DESTRUCTIVE, "text-destructive"),
            ("other", "text-muted-foreground"),
        ] {
            assert_eq!(alert_icon_color_class(variant), expected);
        }

        for (position, expected) in [
            (POSITION_TOP_LEFT, "top-4 left-4"),
            (POSITION_TOP_RIGHT, "top-4 right-4"),
            (POSITION_BOTTOM_LEFT, "bottom-4 left-4"),
            (POSITION_BOTTOM_RIGHT, "bottom-4 right-4"),
            (POSITION_TOP_CENTER, "top-4 left-1/2 -translate-x-1/2"),
            (POSITION_BOTTOM_CENTER, "bottom-4 left-1/2 -translate-x-1/2"),
            ("other", "top-4 right-4"),
        ] {
            assert_eq!(toast_position_classes(position), expected);
        }

        for (position, expected) in [
            (POSITION_TOP, "bottom-full left-1/2 -translate-x-1/2 mb-2"),
            (POSITION_RIGHT, "left-full top-1/2 -translate-y-1/2 ml-2"),
            (POSITION_BOTTOM, "top-full left-1/2 -translate-x-1/2 mt-2"),
            (POSITION_LEFT, "right-full top-1/2 -translate-y-1/2 mr-2"),
            ("other", "bottom-full left-1/2 -translate-x-1/2 mb-2"),
        ] {
            assert_eq!(tooltip_position_classes(position), expected);
        }

        for (value, expected) in [
            (1, "1"),
            (2, "2"),
            (3, "3"),
            (4, "4"),
            (5, "5"),
            (6, "6"),
            (9, "3"),
        ] {
            assert_eq!(int_to_grid_string(value), expected);
        }

        for (shape, expected) in [
            (AVATAR_SHAPE_SQUARE, "rounded-none"),
            (AVATAR_SHAPE_ROUNDED, "rounded-lg"),
            ("other", "rounded-full"),
        ] {
            assert_eq!(avatar_shape_classes(shape), expected);
        }

        for (status, expected) in [
            (AVATAR_STATUS_ONLINE, "bg-success"),
            (AVATAR_STATUS_OFFLINE, "bg-muted-foreground"),
            (AVATAR_STATUS_AWAY, "bg-warning"),
            (AVATAR_STATUS_BUSY, "bg-destructive"),
            ("other", ""),
        ] {
            assert_eq!(avatar_status_classes(status), expected);
        }

        assert_eq!(
            badge_style_classes(BADGE_STYLE_OUTLINE, VARIANT_INFO),
            "border-2 border-info text-info bg-transparent"
        );
        assert_eq!(
            badge_style_classes(BADGE_STYLE_SOFT, VARIANT_INFO),
            "bg-info-muted text-info"
        );
        assert_eq!(
            badge_style_classes("solid", VARIANT_INFO),
            "bg-info text-info-foreground"
        );

        for (variant, expected) in [
            (
                VARIANT_PRIMARY,
                "border-2 border-primary text-primary bg-transparent",
            ),
            (
                VARIANT_SECONDARY,
                "border-2 border-secondary text-secondary bg-transparent",
            ),
            (
                VARIANT_DESTRUCTIVE,
                "border-2 border-destructive text-destructive bg-transparent",
            ),
            (
                VARIANT_SUCCESS,
                "border-2 border-success text-success bg-transparent",
            ),
            (
                VARIANT_WARNING,
                "border-2 border-warning text-warning bg-transparent",
            ),
            (
                VARIANT_INFO,
                "border-2 border-info text-info bg-transparent",
            ),
            (
                "other",
                "border-2 border-border text-foreground bg-transparent",
            ),
        ] {
            assert_eq!(badge_outline_variant_classes(variant), expected);
        }

        for (variant, expected) in [
            (VARIANT_PRIMARY, "bg-primary/15 text-primary"),
            (VARIANT_SECONDARY, "bg-secondary text-secondary-foreground"),
            (VARIANT_DESTRUCTIVE, "bg-destructive-muted text-destructive"),
            (VARIANT_SUCCESS, "bg-success-muted text-success"),
            (VARIANT_WARNING, "bg-warning-muted text-warning"),
            (VARIANT_INFO, "bg-info-muted text-info"),
            ("other", "bg-muted text-muted-foreground"),
        ] {
            assert_eq!(badge_soft_variant_classes(variant), expected);
        }

        for (role, expected) in [
            (CHAT_ROLE_USER, "bg-primary text-primary-foreground ml-auto"),
            (
                CHAT_ROLE_ASSISTANT,
                "bg-muted text-muted-foreground mr-auto",
            ),
            (
                CHAT_ROLE_SYSTEM,
                "bg-secondary/50 text-secondary-foreground mx-auto text-center italic",
            ),
            ("other", "bg-muted text-muted-foreground"),
        ] {
            assert_eq!(chat_role_classes(role), expected);
        }

        for (status, expected) in [
            (STATUS_BADGE_ACTIVE, "bg-success-muted text-success"),
            (STATUS_BADGE_SUCCESS, "bg-success-muted text-success"),
            (STATUS_BADGE_INACTIVE, "bg-muted text-muted-foreground"),
            (STATUS_BADGE_PENDING, "bg-warning-muted text-warning"),
            (STATUS_BADGE_WARNING, "bg-warning-muted text-warning"),
            (STATUS_BADGE_ERROR, "bg-destructive-muted text-destructive"),
            ("other", "bg-muted text-muted-foreground"),
        ] {
            assert_eq!(status_badge_classes(status), expected);
        }

        for (size, expected) in [
            (CONTAINER_SM, "max-w-sm"),
            (CONTAINER_LG, "max-w-4xl"),
            (CONTAINER_XL, "max-w-6xl"),
            (CONTAINER_FULL, "max-w-full"),
            ("other", "max-w-2xl"),
        ] {
            assert_eq!(container_size_classes(size), expected);
        }

        for (weight, expected) in [
            (TEXT_WEIGHT_MEDIUM, "font-medium"),
            (TEXT_WEIGHT_SEMIBOLD, "font-semibold"),
            (TEXT_WEIGHT_BOLD, "font-bold"),
            ("other", "font-normal"),
        ] {
            assert_eq!(text_weight_classes(weight), expected);
        }

        for (color, expected) in [
            (TEXT_COLOR_MUTED, "text-muted-foreground"),
            (TEXT_COLOR_PRIMARY, "text-primary"),
            (TEXT_COLOR_SECONDARY, "text-secondary"),
            (TEXT_COLOR_SUCCESS, "text-success"),
            (TEXT_COLOR_WARNING, "text-warning"),
            (TEXT_COLOR_ERROR, "text-destructive"),
            ("other", "text-foreground"),
        ] {
            assert_eq!(text_color_classes(color), expected);
        }

        for (size, expected) in [
            (SIZE_XS, "text-xs"),
            (SIZE_SM, "text-sm"),
            (SIZE_LG, "text-lg"),
            (SIZE_XL, "text-xl"),
            ("other", "text-base"),
        ] {
            assert_eq!(text_size_classes(size), expected);
        }

        for (variant, expected) in [
            (DIVIDER_DASHED, "border-dashed"),
            (DIVIDER_DOTTED, "border-dotted"),
            ("other", "border-solid"),
        ] {
            assert_eq!(divider_variant_classes(variant), expected);
        }

        for (ratio, expected) in [
            (ASPECT_RATIO_4X3, "aspect-[4/3]"),
            (ASPECT_RATIO_1X1, "aspect-square"),
            (ASPECT_RATIO_9X16, "aspect-[9/16]"),
            ("other", "aspect-video"),
        ] {
            assert_eq!(aspect_ratio_classes(ratio), expected);
        }

        assert_eq!(stack_direction_classes(STACK_HORIZONTAL), "flex-row");
        assert_eq!(stack_direction_classes("other"), "flex-col");

        for (align, expected) in [
            (STACK_ALIGN_START, "items-start"),
            (STACK_ALIGN_END, "items-end"),
            (STACK_ALIGN_STRETCH, "items-stretch"),
            ("other", "items-center"),
        ] {
            assert_eq!(stack_align_classes(align), expected);
        }

        for (justify, expected) in [
            (STACK_JUSTIFY_START, "justify-start"),
            (STACK_JUSTIFY_END, "justify-end"),
            (STACK_JUSTIFY_BETWEEN, "justify-between"),
            (STACK_JUSTIFY_AROUND, "justify-around"),
            ("other", "justify-center"),
        ] {
            assert_eq!(stack_justify_classes(justify), expected);
        }

        for (gap, expected) in [
            (0, "gap-0"),
            (1, "gap-1"),
            (2, "gap-2"),
            (3, "gap-3"),
            (4, "gap-4"),
            (5, "gap-5"),
            (6, "gap-6"),
            (8, "gap-8"),
            (10, "gap-10"),
            (12, "gap-12"),
            (7, "gap-4"),
        ] {
            assert_eq!(gap_classes(gap), expected);
        }

        for (level, expected) in [
            (HEADING_H1, "text-4xl font-bold tracking-tight"),
            (HEADING_H2, "text-3xl font-semibold tracking-tight"),
            (HEADING_H3, "text-2xl font-semibold"),
            (HEADING_H4, "text-xl font-semibold"),
            (HEADING_H5, "text-lg font-medium"),
            (HEADING_H6, "text-base font-medium"),
            ("other", "text-2xl font-semibold"),
        ] {
            assert_eq!(heading_level_classes(level), expected);
        }

        for (variant, expected) in [
            (SKELETON_CARD, "rounded-lg"),
            (SKELETON_LIST, "rounded-md"),
            (SKELETON_TABLE, "rounded-sm"),
            (SKELETON_AVATAR, "rounded-full"),
            (SKELETON_TEXT, "rounded-sm h-4"),
            ("other", "rounded-md"),
        ] {
            assert_eq!(skeleton_variant_classes(variant), expected);
        }

        for (align, expected) in [
            (CARD_FOOTER_START, "justify-start"),
            (CARD_FOOTER_END, "justify-end"),
            (CARD_FOOTER_BETWEEN, "justify-between"),
            ("other", "justify-center"),
        ] {
            assert_eq!(card_footer_align_classes(align), expected);
        }

        for (size, expected) in [
            (SIZE_XS, "h-4 w-7"),
            (SIZE_SM, "h-5 w-9"),
            (SIZE_LG, "h-7 w-[52px]"),
            (SIZE_XL, "h-8 w-[60px]"),
            ("other", "h-6 w-11"),
        ] {
            assert_eq!(switch_size_classes(size), expected);
        }

        for (size, expected) in [
            (SIZE_XS, "h-3 w-3 data-[state=checked]:translate-x-3"),
            (SIZE_SM, "h-4 w-4 data-[state=checked]:translate-x-4"),
            (SIZE_LG, "h-6 w-6 data-[state=checked]:translate-x-[25px]"),
            (SIZE_XL, "h-7 w-7 data-[state=checked]:translate-x-[29px]"),
            ("other", "h-5 w-5 data-[state=checked]:translate-x-5"),
        ] {
            assert_eq!(switch_thumb_size_classes(size), expected);
        }

        for (position, expected) in [
            (POSITION_TOP, "animate-tooltip-in-top"),
            (POSITION_BOTTOM, "animate-tooltip-in-bottom"),
            (POSITION_LEFT, "animate-tooltip-in-left"),
            (POSITION_RIGHT, "animate-tooltip-in-right"),
            ("other", "animate-tooltip-in-top"),
        ] {
            assert_eq!(tooltip_animation_classes(position), expected);
        }

        for (position, expected) in [
            (POSITION_TOP_LEFT, "animate-toast-in-left"),
            (POSITION_BOTTOM_LEFT, "animate-toast-in-left"),
            (POSITION_TOP_RIGHT, "animate-toast-in-right"),
            (POSITION_BOTTOM_RIGHT, "animate-toast-in-right"),
            (POSITION_TOP_CENTER, "animate-toast-in-top"),
            (POSITION_BOTTOM_CENTER, "animate-toast-in-bottom"),
            ("other", "animate-toast-in-right"),
        ] {
            assert_eq!(toast_slide_in_classes(position), expected);
        }

        for (variant, expected) in [
            (VARIANT_INFO, "toast-accent-info"),
            (VARIANT_SUCCESS, "toast-accent-success"),
            (VARIANT_WARNING, "toast-accent-warning"),
            (VARIANT_DESTRUCTIVE, "toast-accent-destructive"),
            ("other", "toast-accent-default"),
        ] {
            assert_eq!(toast_accent_classes(variant), expected);
        }

        for (variant, expected) in [
            (VARIANT_INFO, "toast-progress-bar toast-progress-info"),
            (VARIANT_SUCCESS, "toast-progress-bar toast-progress-success"),
            (VARIANT_WARNING, "toast-progress-bar toast-progress-warning"),
            (
                VARIANT_DESTRUCTIVE,
                "toast-progress-bar toast-progress-destructive",
            ),
            ("other", "toast-progress-bar toast-progress-default"),
        ] {
            assert_eq!(toast_progress_classes(variant), expected);
        }

        for (side, expected) in [
            (MENU_SIDE_TOP, "bottom-full mb-2"),
            (MENU_SIDE_RIGHT, "left-full ml-2"),
            (MENU_SIDE_LEFT, "right-full mr-2"),
            ("other", "top-full mt-2"),
        ] {
            assert_eq!(menu_side_classes(side), expected);
        }

        for (align, expected) in [
            (MENU_ALIGN_END, "right-0"),
            (MENU_ALIGN_CENTER, "left-1/2 -translate-x-1/2"),
            ("other", "left-0"),
        ] {
            assert_eq!(menu_align_classes(align), expected);
        }
    }
}
