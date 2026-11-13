#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "gamificationIcon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "ScoreBadge",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "StatusBadge",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "DifficultyBadge",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "SimpleScoreBadge",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "SimpleStatusBadge",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "SimpleDifficultyBadge",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "GradientStatCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "SimpleGradientStatCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "SuccessGradientStatCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "PrimaryGradientStatCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "WarningGradientStatCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "ItemCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "SimpleItemCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "EarnedItemCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "LockedItemCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "SimpleLockedItemCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "ActionItemCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "StarRating",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "SimpleStarRating",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "StarRatingWithLabel",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "FileItemCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "SimpleFileItemCard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "NotificationPill",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "NotificationPillContainer",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "LevelUpOverlay",
    },
    ComponentInfo {
        family: "unstyled",
        module: "gamification",
        name: "FileItemCardWithAction",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusBadgeProps {
    pub status: String,
    pub label: String,
    pub show_dot: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for StatusBadgeProps {
    fn default() -> Self {
        Self {
            status: "offline".to_string(),
            label: String::new(),
            show_dot: true,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl StatusBadgeProps {
    #[must_use]
    pub fn with_status(mut self, value: impl Into<String>) -> Self {
        self.status = value.into();
        self
    }
    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
        self
    }
    #[must_use]
    pub const fn with_show_dot(mut self, value: bool) -> Self {
        self.show_dot = value;
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
            "inline-flex items-center gap-2 px-2 py-0.5 rounded-full text-xs font-medium",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
    #[must_use]
    pub fn get_dot_color(&self) -> &'static str {
        match self.status.as_str() {
            "online" => "bg-success",
            "away" => "bg-warning",
            "busy" => "bg-destructive",
            _ => "bg-muted-foreground",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScoreBadgeProps {
    pub score: i32,
    pub max_score: i32,
    pub show_max: bool,
    pub variant: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for ScoreBadgeProps {
    fn default() -> Self {
        Self {
            score: 0,
            max_score: 100,
            show_max: true,
            variant: shared::VARIANT_PRIMARY,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl ScoreBadgeProps {
    #[must_use]
    pub const fn with_score(mut self, score: i32, max_score: i32) -> Self {
        self.score = score;
        self.max_score = max_score;
        self
    }
    #[must_use]
    pub const fn with_show_max(mut self, value: bool) -> Self {
        self.show_max = value;
        self
    }
    #[must_use]
    pub const fn with_variant(mut self, value: &'static str) -> Self {
        self.variant = value;
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
            "inline-flex items-center gap-1 px-2.5 py-1 rounded-full text-sm font-semibold",
            shared::badge_variant_classes(self.variant),
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
    #[must_use]
    pub const fn get_score_percent(&self) -> i32 {
        if self.max_score == 0 {
            0
        } else {
            (self.score * 100) / self.max_score
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DifficultyBadgeProps {
    pub level: String,
    pub show_icon: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for DifficultyBadgeProps {
    fn default() -> Self {
        Self {
            level: "medium".to_string(),
            show_icon: true,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl DifficultyBadgeProps {
    #[must_use]
    pub fn with_level(mut self, value: impl Into<String>) -> Self {
        self.level = value.into();
        self
    }
    #[must_use]
    pub const fn with_show_icon(mut self, value: bool) -> Self {
        self.show_icon = value;
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
        let color = match self.level.as_str() {
            "easy" => "bg-success/10 text-success",
            "medium" => "bg-warning/10 text-warning",
            "hard" => "bg-destructive/10 text-destructive",
            "expert" => "bg-primary/10 text-primary",
            _ => "bg-muted text-muted-foreground",
        };
        shared::build_classes(&[
            "inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium",
            color,
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
    #[must_use]
    pub fn get_icon(&self) -> &'static str {
        match self.level.as_str() {
            "easy" => "activity",
            "medium" => "star",
            "hard" => "fire",
            "expert" => "diamond",
            _ => "bar_chart",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GradientStatCardProps {
    pub label: String,
    pub value: String,
    pub icon: String,
    pub gradient_from: String,
    pub gradient_to: String,
    pub gradient_class: String,
    pub text_class: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for GradientStatCardProps {
    fn default() -> Self {
        Self {
            label: String::new(),
            value: String::new(),
            icon: String::new(),
            gradient_from: String::new(),
            gradient_to: String::new(),
            gradient_class: String::new(),
            text_class: "text-primary-foreground".to_string(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl GradientStatCardProps {
    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
        self
    }
    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }
    #[must_use]
    pub fn with_icon(mut self, value: impl Into<String>) -> Self {
        self.icon = value.into();
        self
    }
    #[must_use]
    pub fn with_gradient(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.gradient_from = from.into();
        self.gradient_to = to.into();
        self
    }
    #[must_use]
    pub fn with_gradient_class(mut self, value: impl Into<String>) -> Self {
        self.gradient_class = value.into();
        self
    }
    #[must_use]
    pub fn with_text_class(mut self, value: impl Into<String>) -> Self {
        self.text_class = value.into();
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
    pub fn build_gradient_class(&self) -> String {
        if !self.gradient_class.is_empty() {
            self.gradient_class.clone()
        } else if !self.gradient_from.is_empty() && !self.gradient_to.is_empty() {
            format!(
                "bg-gradient-to-r from-{} to-{}",
                self.gradient_from, self.gradient_to
            )
        } else {
            "bg-gradient-to-r from-primary to-primary/80".to_string()
        }
    }
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "rounded-xl border-0 interactive-card",
            self.build_gradient_class().as_str(),
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_container_classes(&self) -> String {
        self.build_classes()
    }
    #[must_use]
    pub fn build_value_classes(&self) -> String {
        shared::build_classes(&["text-4xl font-bold tabular-nums", self.text_class.as_str()])
    }
    #[must_use]
    pub fn build_label_classes(&self) -> String {
        shared::build_classes(&["text-sm opacity-90", self.text_class.as_str()])
    }
    #[must_use]
    pub fn build_icon_classes(&self) -> String {
        shared::build_classes(&[
            "inline-flex h-8 w-8 items-center justify-center opacity-80",
            self.text_class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemCardProps {
    pub id: String,
    pub title: String,
    pub description: String,
    pub icon: String,
    pub value: i32,
    pub value_label: String,
    pub progress: i32,
    pub status: String,
    pub status_label: String,
    pub timestamp: String,
    pub variant: &'static str,
    pub show_icon: bool,
    pub grayscale: bool,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for ItemCardProps {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            description: String::new(),
            icon: String::new(),
            value: 0,
            value_label: String::new(),
            progress: 0,
            status: String::new(),
            status_label: String::new(),
            timestamp: String::new(),
            variant: shared::VARIANT_PRIMARY,
            show_icon: true,
            grayscale: false,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl ItemCardProps {
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }
    #[must_use]
    pub fn with_description(mut self, value: impl Into<String>) -> Self {
        self.description = value.into();
        self
    }
    #[must_use]
    pub fn with_icon(mut self, value: impl Into<String>) -> Self {
        self.icon = value.into();
        self
    }
    #[must_use]
    pub const fn with_value(mut self, value: i32) -> Self {
        self.value = value;
        self
    }
    #[must_use]
    pub fn with_value_label(mut self, value: impl Into<String>) -> Self {
        self.value_label = value.into();
        self
    }
    #[must_use]
    pub const fn with_progress(mut self, value: i32) -> Self {
        self.progress = value;
        self
    }
    #[must_use]
    pub fn with_status(mut self, value: impl Into<String>) -> Self {
        self.status = value.into();
        self
    }
    #[must_use]
    pub fn with_status_label(mut self, value: impl Into<String>) -> Self {
        self.status_label = value.into();
        self
    }
    #[must_use]
    pub fn with_timestamp(mut self, value: impl Into<String>) -> Self {
        self.timestamp = value.into();
        self
    }
    #[must_use]
    pub const fn with_show_icon(mut self, value: bool) -> Self {
        self.show_icon = value;
        self
    }
    #[must_use]
    pub const fn with_grayscale(mut self, value: bool) -> Self {
        self.grayscale = value;
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
    pub fn is_earned(&self) -> bool {
        matches!(self.status.as_str(), "earned" | "completed" | "unlocked")
    }
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.status == "locked"
    }
    #[must_use]
    pub fn is_pending(&self) -> bool {
        matches!(self.status.as_str(), "pending" | "requested")
    }
    #[must_use]
    pub fn build_icon_class(&self) -> String {
        shared::build_classes(&[
            "inline-flex h-10 w-10 shrink-0 items-center justify-center text-foreground",
            if self.grayscale || self.is_locked() {
                "grayscale"
            } else {
                ""
            },
        ])
    }
    #[must_use]
    pub fn build_card_class(&self) -> String {
        shared::build_classes(&[
            "rounded-xl border border-border bg-card interactive-card",
            if self.is_locked() {
                "bg-muted/50 opacity-75"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockedItemCardProps {
    pub title: String,
    pub description: String,
    pub icon: String,
    pub requirement: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for LockedItemCardProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            icon: "lock".to_string(),
            requirement: String::new(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl LockedItemCardProps {
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }
    #[must_use]
    pub fn with_description(mut self, value: impl Into<String>) -> Self {
        self.description = value.into();
        self
    }
    #[must_use]
    pub fn with_icon(mut self, value: impl Into<String>) -> Self {
        self.icon = value.into();
        self
    }
    #[must_use]
    pub fn with_requirement(mut self, value: impl Into<String>) -> Self {
        self.requirement = value.into();
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
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActionItemCardProps {
    pub item: ItemCardProps,
    pub action_label: String,
    pub action_href: String,
    pub disabled: bool,
}

impl ActionItemCardProps {
    #[must_use]
    pub fn with_item(mut self, value: ItemCardProps) -> Self {
        self.item = value;
        self
    }
    #[must_use]
    pub fn with_action_label(mut self, value: impl Into<String>) -> Self {
        self.action_label = value.into();
        self
    }
    #[must_use]
    pub fn with_action_href(mut self, value: impl Into<String>) -> Self {
        self.action_href = value.into();
        self
    }
    #[must_use]
    pub const fn with_disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StarRatingProps {
    pub stars: i32,
    pub max_stars: i32,
    pub size: &'static str,
    pub filled_char: String,
    pub empty_char: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for StarRatingProps {
    fn default() -> Self {
        Self {
            stars: 0,
            max_stars: 5,
            size: shared::SIZE_MD,
            filled_char: "★".to_string(),
            empty_char: "☆".to_string(),
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl StarRatingProps {
    #[must_use]
    pub const fn with_stars(mut self, value: i32) -> Self {
        self.stars = value;
        self
    }
    #[must_use]
    pub const fn with_max_stars(mut self, value: i32) -> Self {
        self.max_stars = value;
        self
    }
    #[must_use]
    pub fn with_chars(mut self, filled: impl Into<String>, empty: impl Into<String>) -> Self {
        self.filled_char = filled.into();
        self.empty_char = empty.into();
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
        shared::build_classes(&["text-warning", self.class.as_str()])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileItemCardProps {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub file_type: String,
    pub file_size: String,
    pub timestamp: String,
    pub progress: i32,
    pub progress_label: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl FileItemCardProps {
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }
    #[must_use]
    pub fn with_icon(mut self, value: impl Into<String>) -> Self {
        self.icon = value.into();
        self
    }
    #[must_use]
    pub fn with_file_type(mut self, value: impl Into<String>) -> Self {
        self.file_type = value.into();
        self
    }
    #[must_use]
    pub fn with_file_size(mut self, value: impl Into<String>) -> Self {
        self.file_size = value.into();
        self
    }
    #[must_use]
    pub fn with_timestamp(mut self, value: impl Into<String>) -> Self {
        self.timestamp = value.into();
        self
    }
    #[must_use]
    pub const fn with_progress(mut self, value: i32) -> Self {
        self.progress = value;
        self
    }
    #[must_use]
    pub fn with_progress_label(mut self, value: impl Into<String>) -> Self {
        self.progress_label = value.into();
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
    pub fn get_icon(&self) -> String {
        if !self.icon.is_empty() {
            return self.icon.clone();
        }
        match self.file_type.as_str() {
            "pdf" => "📕",
            "doc" | "docx" => "📘",
            "txt" => "📄",
            "xls" | "xlsx" => "📊",
            "ppt" | "pptx" => "📽️",
            "jpg" | "jpeg" | "png" | "gif" => "🖼️",
            "mp3" | "wav" => "🎵",
            "mp4" | "mov" | "avi" => "🎬",
            _ => "📁",
        }
        .to_string()
    }
    #[must_use]
    pub const fn get_progress_variant(&self) -> &'static str {
        if self.progress >= 80 {
            shared::VARIANT_SUCCESS
        } else if self.progress >= 50 {
            shared::VARIANT_WARNING
        } else if self.progress >= 25 {
            shared::VARIANT_INFO
        } else {
            shared::VARIANT_DESTRUCTIVE
        }
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct NotificationPillProps {
    pub id: String,
    pub content: String,
    pub icon_emoji: String,
    pub icon_svg: Option<Markup>,
    pub color_class: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl NotificationPillProps {
    #[must_use]
    pub fn new() -> Self {
        Self {
            color_class: "bg-stat-xp text-stat-xp-foreground".to_string(),
            ..Self::default()
        }
    }
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub fn with_content(mut self, value: impl Into<String>) -> Self {
        self.content = value.into();
        self
    }
    #[must_use]
    pub fn with_icon_emoji(mut self, value: impl Into<String>) -> Self {
        self.icon_emoji = value.into();
        self
    }
    #[must_use]
    pub fn with_icon_svg(mut self, value: Markup) -> Self {
        self.icon_svg = Some(value);
        self
    }
    #[must_use]
    pub fn with_color_class(mut self, value: impl Into<String>) -> Self {
        self.color_class = value.into();
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
            "notification-pill flex items-center font-bold text-sm px-4 py-2 rounded-full shadow-lg",
            self.color_class.as_str(),
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("role".to_string(), "status".to_string());
        attrs.insert("aria-live".to_string(), "polite".to_string());
        attrs
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LevelUpOverlayProps {
    pub id: String,
    pub title: String,
    pub level: String,
    pub subtitle: String,
    pub dismiss_label: String,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl LevelUpOverlayProps {
    #[must_use]
    pub fn with_id(mut self, value: impl Into<String>) -> Self {
        self.id = value.into();
        self
    }
    #[must_use]
    pub fn with_title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }
    #[must_use]
    pub fn with_level(mut self, value: impl Into<String>) -> Self {
        self.level = value.into();
        self
    }
    #[must_use]
    pub fn with_subtitle(mut self, value: impl Into<String>) -> Self {
        self.subtitle = value.into();
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
            "fixed inset-0 z-50 flex items-center justify-center",
            self.class.as_str(),
        ])
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        let mut attrs = self.attributes.clone();
        attrs.insert("role".to_string(), "dialog".to_string());
        if !self.title.is_empty() {
            attrs.insert("aria-label".to_string(), self.title.clone());
        }
        attrs
    }
}

fn attr<'a>(attrs: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    shared::attr_value(attrs, name)
}

#[must_use]
pub fn gamificationIcon(value: &str, class: &str, _size: &str) -> Markup {
    if value.is_empty() {
        html! {}
    } else {
        html! { span class=[shared::optional_attr(class)] data-icon=(value) { (value) } }
    }
}

#[must_use]
pub fn ScoreBadge(props: &ScoreBadgeProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        span class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            (props.score)
            @if props.show_max {
                span class="opacity-70" { "/" (props.max_score) }
            }
        }
    }
}

#[must_use]
pub fn StatusBadge(props: &StatusBadgeProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        span class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            @if props.show_dot {
                span class=(shared::build_classes(&["w-2 h-2 rounded-full", props.get_dot_color()])) {}
            }
            @if props.label.is_empty() { (&props.status) } @else { (&props.label) }
        }
    }
}

#[must_use]
pub fn DifficultyBadge(props: &DifficultyBadgeProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        span class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            @if props.show_icon {
                (gamificationIcon(props.get_icon(), "h-3.5 w-3.5", shared::SIZE_SM))
            }
            span class="capitalize" { (&props.level) }
        }
    }
}

#[must_use]
pub fn SimpleScoreBadge(score: i32, max_score: i32, class: &str) -> Markup {
    ScoreBadge(
        &ScoreBadgeProps::default()
            .with_score(score, max_score)
            .with_class(class),
    )
}
#[must_use]
pub fn SimpleStatusBadge(status: &str, class: &str) -> Markup {
    StatusBadge(
        &StatusBadgeProps::default()
            .with_status(status)
            .with_class(class),
    )
}
#[must_use]
pub fn SimpleDifficultyBadge(level: &str, class: &str) -> Markup {
    DifficultyBadge(
        &DifficultyBadgeProps::default()
            .with_level(level)
            .with_class(class),
    )
}

#[must_use]
pub fn GradientStatCard(props: &GradientStatCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_container_classes()) data-testid=[attr(&attrs, "data-testid")] {
            div class="p-6" {
                div class="flex items-center justify-between" {
                    div class="flex flex-col gap-1" {
                        span class=(props.build_value_classes()) { (&props.value) }
                        span class=(props.build_label_classes()) { (&props.label) }
                    }
                    @if !props.icon.is_empty() {
                        (gamificationIcon(&props.icon, &props.build_icon_classes(), shared::SIZE_XL))
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleGradientStatCard(label: &str, value: &str, icon: &str, gradient: &str) -> Markup {
    GradientStatCard(
        &GradientStatCardProps::default()
            .with_label(label)
            .with_value(value)
            .with_icon(icon)
            .with_gradient_class(gradient)
            .with_text_class("text-primary-foreground"),
    )
}
#[must_use]
pub fn SuccessGradientStatCard(label: &str, value: &str, icon: &str) -> Markup {
    GradientStatCard(
        &GradientStatCardProps::default()
            .with_label(label)
            .with_value(value)
            .with_icon(icon)
            .with_gradient_class("bg-stat-xp")
            .with_text_class("text-stat-xp-foreground"),
    )
}
#[must_use]
pub fn PrimaryGradientStatCard(label: &str, value: &str, icon: &str) -> Markup {
    GradientStatCard(
        &GradientStatCardProps::default()
            .with_label(label)
            .with_value(value)
            .with_icon(icon)
            .with_gradient_class("bg-stat-level")
            .with_text_class("text-stat-level-foreground"),
    )
}
#[must_use]
pub fn WarningGradientStatCard(label: &str, value: &str, icon: &str) -> Markup {
    GradientStatCard(
        &GradientStatCardProps::default()
            .with_label(label)
            .with_value(value)
            .with_icon(icon)
            .with_gradient_class("bg-stat-streak")
            .with_text_class("text-stat-streak-foreground"),
    )
}

#[must_use]
pub fn itemProgressBarClass(progress: i32) -> String {
    let color = if progress >= 80 {
        "bg-success"
    } else if progress >= 50 {
        "bg-warning"
    } else if progress >= 25 {
        "bg-info"
    } else {
        "bg-destructive"
    };
    shared::build_classes(&["h-full rounded-full transition-all duration-300", color])
}

#[must_use]
pub fn ItemCard(props: &ItemCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_card_class()) data-testid=[attr(&attrs, "data-testid")] {
            div class="p-6" {
                div class="flex items-start gap-4" {
                    @if props.show_icon && !props.icon.is_empty() {
                        (gamificationIcon(&props.icon, &props.build_icon_class(), shared::SIZE_XL))
                    }
                    div class="flex-1 min-w-0" {
                        h3 class="font-semibold text-base" { (&props.title) }
                        @if !props.description.is_empty() {
                            p class="text-sm text-muted-foreground mt-1" { (&props.description) }
                        }
                        div class="flex items-center justify-between mt-2" {
                            @if props.value > 0 {
                                span class="text-sm font-medium text-primary" { "+" (props.value) " " (&props.value_label) }
                            }
                            @if !props.timestamp.is_empty() && props.is_earned() {
                                span class="text-xs text-muted-foreground" { "Earned " (&props.timestamp) }
                            }
                        }
                        @if props.progress > 0 && !props.is_earned() {
                            div class="mt-2" {
                                div class="h-2 bg-muted rounded-full overflow-hidden" {
                                    div class=(itemProgressBarClass(props.progress)) style=(format!("width: {}%", props.progress)) {}
                                }
                                span class="text-xs text-muted-foreground mt-1" { (props.progress) "% complete" }
                            }
                        }
                        @if props.is_pending() && !props.status_label.is_empty() {
                            span class="text-xs text-muted-foreground mt-2 block" { (&props.status_label) }
                        }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleItemCard(
    title: &str,
    description: &str,
    icon: &str,
    value: i32,
    status: &str,
) -> Markup {
    ItemCard(
        &ItemCardProps::default()
            .with_title(title)
            .with_description(description)
            .with_icon(icon)
            .with_value(value)
            .with_value_label("XP")
            .with_status(status),
    )
}
#[must_use]
pub fn EarnedItemCard(
    title: &str,
    description: &str,
    icon: &str,
    value: i32,
    earned_at: &str,
) -> Markup {
    ItemCard(
        &ItemCardProps::default()
            .with_title(title)
            .with_description(description)
            .with_icon(icon)
            .with_value(value)
            .with_value_label("XP")
            .with_status("earned")
            .with_timestamp(earned_at),
    )
}

#[must_use]
pub fn LockedItemCard(props: &LockedItemCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["rounded-xl border border-border bg-card", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="p-6" {
                div class="flex items-center justify-between" {
                    span class="w-12 h-12 bg-muted rounded-full inline-flex items-center justify-center" {
                        (gamificationIcon(&props.icon, "h-6 w-6 text-muted-foreground grayscale", shared::SIZE_LG))
                    }
                    span class="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-muted text-muted-foreground border-0" { "Locked" }
                }
                h3 class="font-semibold text-base mt-3" { (&props.title) }
                @if !props.description.is_empty() { p class="text-sm text-muted-foreground mt-1" { (&props.description) } }
                @if !props.requirement.is_empty() {
                    div class="flex items-center gap-1 text-sm text-muted-foreground mt-3" {
                        (gamificationIcon("check", "h-4 w-4 text-success", shared::SIZE_SM))
                        span { "Requires " (&props.requirement) }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleLockedItemCard(
    title: &str,
    description: &str,
    icon: &str,
    requirement: &str,
) -> Markup {
    LockedItemCard(
        &LockedItemCardProps::default()
            .with_title(title)
            .with_description(description)
            .with_icon(icon)
            .with_requirement(requirement),
    )
}

#[must_use]
pub fn ActionItemCard(props: &ActionItemCardProps, children: Markup) -> Markup {
    let attrs = props.item.build_attributes();
    html! {
        div class=(props.item.build_card_class()) data-testid=[attr(&attrs, "data-testid")] {
            div class="p-6" {
                div class="flex flex-col gap-4" {
                    div class="flex items-start gap-4" {
                        @if props.item.show_icon && !props.item.icon.is_empty() {
                            (gamificationIcon(&props.item.icon, &props.item.build_icon_class(), shared::SIZE_XL))
                        }
                        div class="flex-1 min-w-0" {
                            h3 class="font-semibold text-base" { (&props.item.title) }
                            @if !props.item.description.is_empty() { p class="text-sm text-muted-foreground mt-1" { (&props.item.description) } }
                            @if props.item.value > 0 { span class="text-sm font-medium text-primary mt-2 block" { "+" (props.item.value) " " (&props.item.value_label) } }
                        }
                    }
                    div class="action-slot" { (children) }
                }
            }
        }
    }
}

#[must_use]
pub fn StarRating(props: &StarRatingProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        span class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            @for i in 0..props.max_stars {
                @if i < props.stars { (&props.filled_char) } @else { (&props.empty_char) }
            }
        }
    }
}

#[must_use]
pub fn SimpleStarRating(stars: i32, class: &str) -> Markup {
    StarRating(
        &StarRatingProps::default()
            .with_stars(stars)
            .with_class(class),
    )
}
#[must_use]
pub fn StarRatingWithLabel(stars: i32, label: &str, class: &str) -> Markup {
    html! { div class=(shared::build_classes(&["inline-flex items-center gap-2", class])) { (SimpleStarRating(stars, "")) @if !label.is_empty() { span class="text-sm text-muted-foreground" { (label) } } } }
}

#[must_use]
pub fn FileItemCard(props: &FileItemCardProps) -> Markup {
    render_file_item_card(props, None)
}

fn render_file_item_card(props: &FileItemCardProps, action: Option<Markup>) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["rounded-xl border border-border bg-card", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="p-6" {
                div class=(if action.is_some() { "flex items-start justify-between gap-4" } else { "flex items-start justify-between gap-4" }) {
                    div class=(if action.is_some() { "flex items-start gap-4 flex-1" } else { "flex items-start gap-4" }) {
                        (gamificationIcon(&props.get_icon(), "inline-flex h-8 w-8 items-center justify-center text-muted-foreground", shared::SIZE_XL))
                        div class=(if action.is_some() { "flex flex-col gap-1 flex-1" } else { "flex flex-col gap-1" }) {
                            h3 class="font-semibold text-lg" { (&props.title) }
                            div class="flex items-center gap-4 text-sm text-muted-foreground" {
                                @if !props.file_type.is_empty() { span { (&props.file_type) } }
                                @if !props.file_size.is_empty() { span { (&props.file_size) } }
                                @if !props.timestamp.is_empty() { span { (&props.timestamp) } }
                            }
                            @if action.is_some() && props.progress > 0 {
                                div class="mt-2" {
                                    div class="flex items-center justify-between text-sm mb-1" {
                                        span class="text-muted-foreground" { (&props.progress_label) }
                                        span class="font-medium" { (props.progress) "%" }
                                    }
                                    div class="w-full h-2 bg-muted rounded-full overflow-hidden" {
                                        div class=(itemProgressBarClass(props.progress)) style=(format!("width: {}%", props.progress)) {}
                                    }
                                }
                            }
                        }
                    }
                    @if action.is_none() && (props.progress > 0 || !props.progress_label.is_empty()) {
                        div class="flex flex-col items-end gap-1 text-right" {
                            span class="text-sm text-muted-foreground" { (&props.progress_label) }
                            div class="w-32 h-2 bg-muted rounded-full overflow-hidden" { div class=(itemProgressBarClass(props.progress)) style=(format!("width: {}%", props.progress)) {} }
                            span class="text-sm font-medium" { (props.progress) "%" }
                        }
                    }
                    @if let Some(action_markup) = action {
                        div class="action-slot" { (action_markup) }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleFileItemCard(
    title: &str,
    file_type: &str,
    file_size: &str,
    timestamp: &str,
    progress: i32,
) -> Markup {
    FileItemCard(
        &FileItemCardProps::default()
            .with_title(title)
            .with_file_type(file_type)
            .with_file_size(file_size)
            .with_timestamp(timestamp)
            .with_progress(progress)
            .with_progress_label("Progress"),
    )
}

#[must_use]
pub fn NotificationPill(props: &NotificationPillProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            id=[shared::optional_attr(&props.id)]
            class=(props.build_classes())
            role=[attr(&attrs, "role")]
            aria-live=[attr(&attrs, "aria-live")]
            data-testid=[attr(&attrs, "data-testid")]
        {
            @if let Some(icon) = props.icon_svg.clone() {
                span class="mr-1.5" { (icon) }
            } @else if !props.icon_emoji.is_empty() {
                span class="mr-1.5 text-base" { (&props.icon_emoji) }
            }
            (&props.content)
        }
    }
}

#[must_use]
pub fn NotificationPillContainer(class: &str, children: Markup) -> Markup {
    html! { div class=(shared::build_classes(&["fixed top-4 left-1/2 -translate-x-1/2 z-50 flex flex-col items-center gap-2 pointer-events-none", class])) { (children) } }
}

#[must_use]
pub fn levelUpOverlayTitle(props: &LevelUpOverlayProps) -> String {
    if props.title.is_empty() {
        "Level Up!".to_string()
    } else {
        props.title.clone()
    }
}

#[must_use]
pub fn LevelUpOverlay(props: &LevelUpOverlayProps, children: Markup) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            id=[shared::optional_attr(&props.id)]
            class=(props.build_classes())
            style="background: var(--overlay-bg); backdrop-filter: blur(4px);"
            role=[attr(&attrs, "role")]
            aria-label=[attr(&attrs, "aria-label")]
            data-testid=[attr(&attrs, "data-testid")]
        {
            div class="relative p-8 rounded-2xl shadow-xl text-center flex flex-col items-center max-w-sm mx-4 bg-stat-level text-stat-level-foreground" {
                div class="starburst" {}
                div class="starburst" style="animation-delay: 0.15s;" {}
                div class="starburst" style="animation-delay: 0.3s;" {}
                p class="text-sm font-semibold uppercase tracking-widest opacity-80 relative z-10" { (levelUpOverlayTitle(props)) }
                h3 class="text-3xl font-heading font-bold mt-1 relative z-10" { (&props.level) }
                @if !props.subtitle.is_empty() { p class="text-sm opacity-80 mt-2 relative z-10" { (&props.subtitle) } }
                (children)
            }
        }
    }
}

#[must_use]
pub fn FileItemCardWithAction(props: &FileItemCardProps, children: Markup) -> Markup {
    render_file_item_card(props, Some(children))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges_and_icons_cover_helpers_variants_and_rendering() {
        assert_eq!(
            StatusBadgeProps::default().get_dot_color(),
            "bg-muted-foreground"
        );
        assert_eq!(
            StatusBadgeProps::default()
                .with_status("online")
                .get_dot_color(),
            "bg-success"
        );
        assert_eq!(
            StatusBadgeProps::default()
                .with_status("away")
                .get_dot_color(),
            "bg-warning"
        );
        assert_eq!(
            StatusBadgeProps::default()
                .with_status("busy")
                .get_dot_color(),
            "bg-destructive"
        );
        assert_eq!(
            ScoreBadgeProps::default()
                .with_score(25, 0)
                .get_score_percent(),
            0
        );
        assert_eq!(
            ScoreBadgeProps::default()
                .with_score(25, 50)
                .get_score_percent(),
            50
        );
        assert_eq!(
            DifficultyBadgeProps::default()
                .with_level("easy")
                .get_icon(),
            "activity"
        );
        assert_eq!(
            DifficultyBadgeProps::default()
                .with_level("medium")
                .get_icon(),
            "star"
        );
        assert_eq!(
            DifficultyBadgeProps::default()
                .with_level("hard")
                .get_icon(),
            "fire"
        );
        assert_eq!(
            DifficultyBadgeProps::default()
                .with_level("expert")
                .get_icon(),
            "diamond"
        );
        assert_eq!(
            DifficultyBadgeProps::default().with_level("x").get_icon(),
            "bar_chart"
        );
        assert_eq!(gamificationIcon("", "", shared::SIZE_SM).into_string(), "");
        assert!(
            gamificationIcon("<icon>", "icon-class", shared::SIZE_SM)
                .into_string()
                .contains("data-icon=\"&lt;icon&gt;\"")
        );

        let score = ScoreBadge(
            &ScoreBadgeProps::default()
                .with_score(7, 10)
                .with_attribute("data-testid", "score"),
        )
        .into_string();
        assert!(score.contains("data-testid=\"score\""));
        assert!(score.contains(">7<span class=\"opacity-70\">/10</span>"));
        assert!(
            !ScoreBadge(
                &ScoreBadgeProps::default()
                    .with_score(7, 10)
                    .with_show_max(false)
            )
            .into_string()
            .contains("/10")
        );
        let status = StatusBadge(
            &StatusBadgeProps::default()
                .with_status("online")
                .with_label("Online"),
        )
        .into_string();
        assert!(status.contains("bg-success"));
        assert!(status.contains("Online</span>"));
        assert!(
            !StatusBadge(&StatusBadgeProps::default().with_show_dot(false))
                .into_string()
                .contains("w-2 h-2")
        );
        let difficulty =
            DifficultyBadge(&DifficultyBadgeProps::default().with_level("hard")).into_string();
        assert!(difficulty.contains("bg-destructive/10 text-destructive"));
        assert!(difficulty.contains("data-icon=\"fire\""));
        assert!(
            SimpleScoreBadge(1, 2, "simple")
                .into_string()
                .contains("simple")
        );
        assert!(
            SimpleStatusBadge("busy", "busy-class")
                .into_string()
                .contains("busy-class")
        );
        assert!(
            SimpleDifficultyBadge("expert", "expert-class")
                .into_string()
                .contains("expert-class")
        );
    }

    #[test]
    fn gradient_and_item_helpers_cover_classes_progress_and_states() {
        let gradient = GradientStatCardProps::default().with_gradient("success", "success/80");
        assert_eq!(
            gradient.build_gradient_class(),
            "bg-gradient-to-r from-success to-success/80"
        );
        assert!(gradient.build_classes().contains("interactive-card"));
        assert!(gradient.build_value_classes().contains("text-4xl"));
        assert!(gradient.build_label_classes().contains("opacity-90"));
        assert!(gradient.build_icon_classes().contains("h-8 w-8"));
        assert_eq!(
            GradientStatCardProps::default().build_gradient_class(),
            "bg-gradient-to-r from-primary to-primary/80"
        );

        assert_eq!(
            itemProgressBarClass(90),
            "h-full rounded-full transition-all duration-300 bg-success"
        );
        assert_eq!(
            itemProgressBarClass(60),
            "h-full rounded-full transition-all duration-300 bg-warning"
        );
        assert_eq!(
            itemProgressBarClass(30),
            "h-full rounded-full transition-all duration-300 bg-info"
        );
        assert_eq!(
            itemProgressBarClass(10),
            "h-full rounded-full transition-all duration-300 bg-destructive"
        );

        let earned = ItemCardProps::default().with_status("earned");
        assert!(earned.is_earned());
        assert!(!earned.is_locked());
        assert!(!earned.is_pending());
        let locked = ItemCardProps::default().with_status("locked");
        assert!(locked.is_locked());
        assert!(locked.build_card_class().contains("bg-muted/50 opacity-75"));
        assert!(locked.build_icon_class().contains("grayscale"));
        assert!(
            ItemCardProps::default()
                .with_status("requested")
                .is_pending()
        );
        assert!(
            ItemCardProps::default()
                .with_grayscale(true)
                .build_icon_class()
                .contains("grayscale")
        );
    }

    #[test]
    fn cards_star_file_notification_and_overlay_helpers_cover_branches() {
        let file = FileItemCardProps::default().with_file_type("pdf");
        assert_eq!(file.get_icon(), "📕");
        assert_eq!(
            FileItemCardProps::default()
                .with_file_type("docx")
                .get_icon(),
            "📘"
        );
        assert_eq!(
            FileItemCardProps::default()
                .with_file_type("txt")
                .get_icon(),
            "📄"
        );
        assert_eq!(
            FileItemCardProps::default()
                .with_file_type("xlsx")
                .get_icon(),
            "📊"
        );
        assert_eq!(
            FileItemCardProps::default()
                .with_file_type("ppt")
                .get_icon(),
            "📽️"
        );
        assert_eq!(
            FileItemCardProps::default()
                .with_file_type("png")
                .get_icon(),
            "🖼️"
        );
        assert_eq!(
            FileItemCardProps::default()
                .with_file_type("mp3")
                .get_icon(),
            "🎵"
        );
        assert_eq!(
            FileItemCardProps::default()
                .with_file_type("mp4")
                .get_icon(),
            "🎬"
        );
        assert_eq!(
            FileItemCardProps::default()
                .with_file_type("zip")
                .get_icon(),
            "📁"
        );
        assert_eq!(
            FileItemCardProps::default().with_icon("custom").get_icon(),
            "custom"
        );
        assert_eq!(
            FileItemCardProps::default()
                .with_progress(80)
                .get_progress_variant(),
            shared::VARIANT_SUCCESS
        );
        assert_eq!(
            FileItemCardProps::default()
                .with_progress(50)
                .get_progress_variant(),
            shared::VARIANT_WARNING
        );
        assert_eq!(
            FileItemCardProps::default()
                .with_progress(25)
                .get_progress_variant(),
            shared::VARIANT_INFO
        );
        assert_eq!(
            FileItemCardProps::default()
                .with_progress(1)
                .get_progress_variant(),
            shared::VARIANT_DESTRUCTIVE
        );

        let pill = NotificationPillProps::new()
            .with_content("+10 XP")
            .with_icon_emoji("🔥")
            .with_attribute("data-testid", "pill");
        assert!(
            pill.build_classes()
                .contains("bg-stat-xp text-stat-xp-foreground")
        );
        let attrs = pill.build_attributes();
        assert_eq!(attr(&attrs, "role"), Some("status"));
        assert_eq!(attr(&attrs, "aria-live"), Some("polite"));
        assert_eq!(attr(&attrs, "data-testid"), Some("pill"));

        let overlay = LevelUpOverlayProps::default()
            .with_title("Level Up")
            .with_level("Level 2")
            .with_attribute("data-testid", "overlay");
        assert_eq!(
            levelUpOverlayTitle(&LevelUpOverlayProps::default()),
            "Level Up!"
        );
        assert_eq!(levelUpOverlayTitle(&overlay), "Level Up");
        let overlay_attrs = overlay.build_attributes();
        assert_eq!(attr(&overlay_attrs, "role"), Some("dialog"));
        assert_eq!(attr(&overlay_attrs, "aria-label"), Some("Level Up"));
    }

    #[test]
    fn complex_components_render_expected_html_and_slots() {
        let stat = GradientStatCard(
            &GradientStatCardProps::default()
                .with_label("<XP>")
                .with_value("100")
                .with_icon("sparkles")
                .with_gradient_class("bg-stat-xp")
                .with_attribute("data-testid", "stat"),
        )
        .into_string();
        assert!(stat.contains("data-testid=\"stat\""));
        assert!(stat.contains("bg-stat-xp"));
        assert!(stat.contains("&lt;XP&gt;"));
        assert!(stat.contains("data-icon=\"sparkles\""));
        assert!(
            SuccessGradientStatCard("XP", "10", "star")
                .into_string()
                .contains("bg-stat-xp")
        );
        assert!(
            PrimaryGradientStatCard("Level", "2", "trophy")
                .into_string()
                .contains("bg-stat-level")
        );
        assert!(
            WarningGradientStatCard("Streak", "4", "fire")
                .into_string()
                .contains("bg-stat-streak")
        );
        assert!(
            SimpleGradientStatCard("A", "B", "C", "bg-custom")
                .into_string()
                .contains("bg-custom")
        );

        let item = ItemCard(
            &ItemCardProps::default()
                .with_title("<Title>")
                .with_description("<Desc>")
                .with_icon("gift")
                .with_value(10)
                .with_value_label("XP")
                .with_progress(40)
                .with_status("pending")
                .with_status_label("Waiting")
                .with_attribute("data-testid", "item"),
        )
        .into_string();
        assert!(item.contains("data-testid=\"item\""));
        assert!(item.contains("&lt;Title&gt;"));
        assert!(item.contains("&lt;Desc&gt;"));
        assert!(item.contains("+10 XP"));
        assert!(item.contains("width: 40%"));
        assert!(item.contains("Waiting"));
        assert!(
            SimpleItemCard("Simple", "Desc", "gift", 5, "available")
                .into_string()
                .contains("Simple")
        );
        assert!(
            EarnedItemCard("Earned", "Desc", "gift", 5, "today")
                .into_string()
                .contains("Earned today")
        );

        let locked = LockedItemCard(
            &LockedItemCardProps::default()
                .with_title("Locked")
                .with_description("Desc")
                .with_requirement("Level 2")
                .with_attribute("data-testid", "locked"),
        )
        .into_string();
        assert!(locked.contains("data-testid=\"locked\""));
        assert!(locked.contains("Locked</span>"));
        assert!(locked.contains("Requires Level 2"));
        assert!(
            SimpleLockedItemCard("L", "D", "lock", "R")
                .into_string()
                .contains("Requires R")
        );

        let action = ActionItemCard(
            &ActionItemCardProps::default().with_item(
                ItemCardProps::default()
                    .with_title("Action")
                    .with_icon("gift")
                    .with_value(3)
                    .with_value_label("XP"),
            ),
            html! { button { "Claim" } },
        )
        .into_string();
        assert!(action.contains("action-slot"));
        assert!(action.contains("<button>Claim</button>"));

        assert_eq!(
            StarRating(&StarRatingProps::default().with_stars(3)).into_string(),
            "<span class=\"text-warning\">★★★☆☆</span>"
        );
        assert!(SimpleStarRating(2, "stars").into_string().contains("stars"));
        assert!(
            StarRatingWithLabel(4, "Good", "rating")
                .into_string()
                .contains("<span class=\"text-sm text-muted-foreground\">Good</span>")
        );

        let file = FileItemCard(
            &FileItemCardProps::default()
                .with_title("Doc")
                .with_file_type("pdf")
                .with_file_size("1 MB")
                .with_timestamp("today")
                .with_progress(70)
                .with_progress_label("Mastery"),
        )
        .into_string();
        assert!(file.contains("Doc"));
        assert!(file.contains("📕"));
        assert!(file.contains("Mastery"));
        assert!(file.contains("70%"));
        assert!(
            SimpleFileItemCard("Doc", "txt", "1 KB", "today", 20)
                .into_string()
                .contains("Progress")
        );
        let file_action = FileItemCardWithAction(
            &FileItemCardProps::default()
                .with_title("Doc")
                .with_progress(30)
                .with_progress_label("Progress"),
            html! { a href="/open" { "Open" } },
        )
        .into_string();
        assert!(file_action.contains("action-slot"));
        assert!(file_action.contains("<a href=\"/open\">Open</a>"));
    }

    #[test]
    fn notification_and_overlay_components_render_accessible_html() {
        let pill = NotificationPill(
            &NotificationPillProps::new()
                .with_id("pill")
                .with_content("+10 XP")
                .with_icon_svg(html! { svg data-icon="xp" {} }),
        )
        .into_string();
        assert!(pill.starts_with("<div id=\"pill\" class=\"notification-pill"));
        assert!(pill.contains("role=\"status\""));
        assert!(pill.contains("aria-live=\"polite\""));
        assert!(pill.contains("<span class=\"mr-1.5\"><svg data-icon=\"xp\"></svg></span>"));
        assert!(pill.contains("+10 XP</div>"));
        let emoji = NotificationPill(
            &NotificationPillProps::new()
                .with_content("Streak")
                .with_icon_emoji("🔥"),
        )
        .into_string();
        assert!(emoji.contains("<span class=\"mr-1.5 text-base\">🔥</span>"));
        assert!(
            NotificationPillContainer("extra", html! { span { "Pill" } })
                .into_string()
                .contains("pointer-events-none extra")
        );

        let overlay = LevelUpOverlay(
            &LevelUpOverlayProps::default()
                .with_id("level")
                .with_title("Level Up")
                .with_level("Level 2")
                .with_subtitle("Keep going")
                .with_attribute("data-testid", "overlay"),
            html! { button { "Continue" } },
        )
        .into_string();
        assert!(overlay.starts_with("<div id=\"level\" class=\"fixed inset-0 z-50"));
        assert!(overlay.contains("role=\"dialog\""));
        assert!(overlay.contains("aria-label=\"Level Up\""));
        assert!(overlay.contains("data-testid=\"overlay\""));
        assert!(overlay.contains("starburst"));
        assert!(overlay.contains(
            "<h3 class=\"text-3xl font-heading font-bold mt-1 relative z-10\">Level 2</h3>"
        ));
        assert!(overlay.contains("Keep going"));
        assert!(overlay.contains("<button>Continue</button>"));
    }

    #[test]
    fn remaining_builder_methods_and_branch_variations_validate_outputs() {
        let status_props = StatusBadgeProps::default()
            .with_status("offline")
            .with_label("Offline")
            .with_class("status-extra")
            .with_attribute("data-testid", "status");
        assert_eq!(
            status_props
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("status")
        );
        let status_html = StatusBadge(&status_props).into_string();
        assert!(status_html.contains("status-extra"));
        assert!(status_html.contains("Offline</span>"));

        let score_props = ScoreBadgeProps::default()
            .with_score(8, 10)
            .with_show_max(false)
            .with_variant(shared::VARIANT_SUCCESS)
            .with_class("score-extra")
            .with_attribute("data-testid", "score");
        assert!(score_props.build_classes().contains("bg-success"));
        let score_html = ScoreBadge(&score_props).into_string();
        assert!(score_html.contains(r#"data-testid="score""#));
        assert!(score_html.contains(">8</span>"));
        assert!(!score_html.contains("/10"));

        let difficulty_props = DifficultyBadgeProps::default()
            .with_level("unknown")
            .with_show_icon(false)
            .with_class("difficulty-extra")
            .with_attribute("data-testid", "difficulty");
        assert_eq!(difficulty_props.get_icon(), "bar_chart");
        assert!(difficulty_props.build_classes().contains("bg-muted"));
        let difficulty_html = DifficultyBadge(&difficulty_props).into_string();
        assert!(difficulty_html.contains(r#"data-testid="difficulty""#));
        assert!(difficulty_html.contains("difficulty-extra"));
        assert!(!difficulty_html.contains("data-icon"));

        let gradient_props = GradientStatCardProps::default()
            .with_label("Label")
            .with_value("9")
            .with_icon("spark")
            .with_gradient("success", "warning")
            .with_class("gradient-extra")
            .with_attribute("data-testid", "gradient");
        assert!(gradient_props.build_classes().contains("gradient-extra"));
        let gradient_html = GradientStatCard(&gradient_props).into_string();
        assert!(gradient_html.contains(r#"data-testid="gradient""#));
        assert!(gradient_html.contains("from-success to-warning"));

        let item_props = ItemCardProps::default()
            .with_title("No icon")
            .with_description("Hidden icon")
            .with_icon("gift")
            .with_value(1)
            .with_value_label("XP")
            .with_progress(90)
            .with_status("unlocked")
            .with_show_icon(false)
            .with_grayscale(true)
            .with_class("item-extra")
            .with_attribute("data-testid", "item");
        assert!(item_props.is_earned());
        assert!(item_props.build_card_class().contains("item-extra"));
        let item_html = ItemCard(&item_props).into_string();
        assert!(item_html.contains(r#"data-testid="item""#));
        assert!(!item_html.contains(r#"data-icon="gift""#));
        assert!(!item_html.contains("% complete"));

        let locked_props = LockedItemCardProps::default()
            .with_title("Locked")
            .with_description("Desc")
            .with_icon("lock")
            .with_requirement("Level 2")
            .with_class("locked-extra")
            .with_attribute("data-testid", "locked");
        assert_eq!(
            locked_props
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("locked")
        );
        let locked_html = LockedItemCard(&locked_props).into_string();
        assert!(locked_html.contains("locked-extra"));
        assert!(locked_html.contains("Requires Level 2"));

        let action_props = ActionItemCardProps::default()
            .with_item(ItemCardProps::default().with_title("Action"))
            .with_action_label("Claim")
            .with_action_href("/claim")
            .with_disabled(true);
        assert_eq!(action_props.action_label, "Claim");
        assert_eq!(action_props.action_href, "/claim");
        assert!(action_props.disabled);
        let action_html = ActionItemCard(
            &action_props,
            html! { a href=(action_props.action_href) { (action_props.action_label) } },
        )
        .into_string();
        assert!(action_html.contains(r#"<a href="/claim">Claim</a>"#));

        let stars = StarRatingProps::default()
            .with_stars(2)
            .with_max_stars(4)
            .with_chars("x", "-")
            .with_class("star-extra")
            .with_attribute("data-testid", "stars");
        assert_eq!(
            stars
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("stars")
        );
        assert_eq!(
            StarRating(&stars).into_string(),
            r#"<span class="text-warning star-extra" data-testid="stars">xx--</span>"#
        );

        let file_props = FileItemCardProps::default()
            .with_title("Sheet")
            .with_icon("custom-file")
            .with_file_type("xls")
            .with_file_size("4 KB")
            .with_timestamp("today")
            .with_progress(10)
            .with_progress_label("Uploaded")
            .with_class("file-extra")
            .with_attribute("data-testid", "file");
        assert_eq!(file_props.get_icon(), "custom-file");
        let file_html = FileItemCard(&file_props).into_string();
        assert!(file_html.contains("file-extra"));
        assert!(file_html.contains(r#"data-testid="file""#));
        assert!(file_html.contains("Uploaded"));

        let pill_props = NotificationPillProps::new()
            .with_id("pill")
            .with_content("Content")
            .with_color_class("bg-custom text-custom")
            .with_class("pill-extra")
            .with_attribute("data-testid", "pill");
        assert!(pill_props.build_classes().contains("bg-custom text-custom"));
        let pill_html = NotificationPill(&pill_props).into_string();
        assert!(pill_html.contains("pill-extra"));
        assert!(pill_html.contains(r#"data-testid="pill""#));

        let overlay_props = LevelUpOverlayProps::default()
            .with_id("overlay")
            .with_level("3")
            .with_subtitle("Done")
            .with_class("overlay-extra")
            .with_attribute("data-testid", "overlay");
        assert!(overlay_props.build_classes().contains("overlay-extra"));
        let overlay_html = LevelUpOverlay(&overlay_props, html! {}).into_string();
        assert!(overlay_html.contains("Level Up!"));
        assert!(overlay_html.contains(r#"data-testid="overlay""#));
    }
}
