#![allow(non_snake_case)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

type Attrs = BTreeMap<String, String>;

const fn component(name: &'static str) -> ComponentInfo {
    ComponentInfo {
        family: "unstyled",
        module: "indicators",
        name,
    }
}

pub const COMPONENTS: &[ComponentInfo] = &[
    component("OnlineIndicator"),
    component("NotificationBell"),
    component("MobileMenuButton"),
    component("AnimatedProgress"),
    component("SimpleOnlineIndicator"),
    component("SimpleNotificationBell"),
    component("SimpleMobileMenuButton"),
    component("SimpleAnimatedProgress"),
    component("Spinner"),
    component("SimpleSpinner"),
    component("SVGSpinner"),
    component("SimpleSVGSpinner"),
    component("Timer"),
    component("SimpleTimer"),
    component("SessionTimer"),
    component("QuizTimer"),
    component("GenerationStateIndicator"),
    component("SimpleGenerationStateIndicator"),
    component("ProcessingProgress"),
    component("SimpleProcessingProgress"),
    component("ProcessingSpinnerMessage"),
    component("ProcessingComplete"),
    component("iconOrSymbol"),
    component("ProcessingError"),
    component("StatFragment"),
    component("SimpleStatFragment"),
    component("StatFragmentWithIcon"),
    component("ValidationMessage"),
    component("ValidationError"),
    component("ValidationSuccess"),
    component("ToastNotification"),
    component("ToastSuccess"),
    component("ToastError"),
    component("ToastWarning"),
    component("ToastInfo"),
    component("ActivityItem"),
    component("SimpleActivityItem"),
    component("TypingIndicator"),
    component("TimerDisplay"),
    component("CountBadge"),
    component("InlineNotification"),
    component("inlineNotificationIcon"),
    component("CountBadgeComponent"),
    component("PendingCountBadge"),
    component("CompletedCountBadge"),
    component("UnreadCountBadge"),
    component("PendingQuizzesBadge"),
    component("CompletedQuizzesBadge"),
    component("UnreadMessagesBadge"),
    component("SessionCountBadge"),
    component("DocumentCountBadge"),
    component("FormLevelValidation"),
    component("FormLevelError"),
    component("FormLevelSuccess"),
    component("SimpleFormError"),
    component("SimpleFormSuccess"),
    component("FormValidationSummary"),
    component("XPProgressFragment"),
    component("StreakFragment"),
    component("ScoreFragment"),
    component("OnlineStatusFragment"),
    component("MasteryFragment"),
    component("QuizResultFragment"),
    component("ItemProgress"),
    component("SimpleItemProgress"),
    component("ItemProgressWithBadge"),
    component("FileProcessingItem"),
    component("GenerationProgress"),
    component("SimpleGenerationProgress"),
    component("GenerationInProgress"),
    component("GenerationCompleteWithItems"),
    component("GenerationError"),
    component("ToastContainer"),
    component("ToastSuccessPositioned"),
    component("ToastErrorPositioned"),
    component("StreamingContent"),
    component("SimpleStreamingContent"),
    component("StreamingContentWithIcon"),
    component("ProgressCounter"),
    component("SimpleProgressCounter"),
    component("ProgressCounterWithBar"),
    component("ChatBubble"),
    component("ChatBubbleOutgoing"),
    component("ChatBubbleIncoming"),
    component("StatCard"),
    component("SimpleStatCard"),
    component("StatCardWithTrend"),
    component("NeomorphStatCard"),
    component("Pagination"),
    component("SimplePagination"),
    component("CompactPagination"),
    component("SimpleStat"),
    component("SimpleStatValue"),
    component("SimpleStatAligned"),
    component("IconStatCard"),
    component("SimpleIconStatCard"),
    component("NeomorphIconStatCard"),
    component("UserListItem"),
    component("userListItemContent"),
    component("SimpleUserListItem"),
    component("UserListItemWithStats"),
    component("RankingCard"),
    component("SimpleRankingCard"),
    component("SectionCard"),
    component("SimpleSectionCard"),
    component("SectionCardWithIcon"),
    component("MasteryDisplay"),
    component("SimpleMasteryDisplay"),
    component("MasteryDisplayWithBar"),
    component("ActivityRow"),
    component("SimpleActivityRow"),
    component("TranscriptMessage"),
    component("SimpleTranscriptMessage"),
    component("TranscriptMessageWithLabel"),
    component("ListItemLinkCard"),
    component("SimpleListItemLinkCard"),
    component("ListItemLinkCardWithStatus"),
    component("StatIconRow"),
    component("SimpleStatIconRow"),
    component("StatusBadge"),
    component("SimpleStatusBadge"),
    component("ActiveStatusBadge"),
    component("CompletedStatusBadge"),
    component("PendingStatusBadge"),
    component("InactiveStatusBadge"),
    component("ItemInfoCard"),
    component("SimpleItemInfoCard"),
    component("ItemInfoCardWithMeta"),
    component("TrophyItemCard"),
    component("LockedTrophyItemCard"),
    component("RewardItemCard"),
];

#[must_use]
pub fn optional(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

#[must_use]
pub fn attr<'a>(attrs: &'a Attrs, name: &str) -> Option<&'a str> {
    shared::attr_value(attrs, name)
}

#[must_use]
pub fn boolStr(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

#[must_use]
pub fn percent(value: i32, max: i32) -> i32 {
    if max == 0 { 0 } else { (value * 100) / max }
}

#[must_use]
pub fn clampedPercent(value: i32, max: i32) -> i32 {
    percent(value, max).clamp(0, 100)
}

#[must_use]
pub fn getOnlineLabel(isOnline: bool) -> &'static str {
    if isOnline { "Online" } else { "Offline" }
}

#[must_use]
pub fn onlineLabelKey(isOnline: bool) -> &'static str {
    if isOnline {
        "component.indicators.online"
    } else {
        "component.indicators.offline"
    }
}

#[must_use]
pub fn getOnlineColor(isOnline: bool) -> &'static str {
    if isOnline {
        "bg-success"
    } else {
        "bg-muted-foreground"
    }
}

#[must_use]
pub fn getNotificationLabel(count: i32) -> String {
    match count {
        0 => "No notifications".to_string(),
        1 => "1 notification".to_string(),
        _ => format!("{count} notifications"),
    }
}

#[must_use]
pub fn resolveNotificationLabel(count: i32) -> String {
    getNotificationLabel(count)
}

#[must_use]
pub fn getBellClass(isAnimating: bool) -> &'static str {
    if isAnimating { "animate-wiggle" } else { "" }
}

#[must_use]
pub fn getMenuLineTop(isOpen: bool) -> &'static str {
    if isOpen {
        "top-1/2 -translate-y-1/2 rotate-45"
    } else {
        "top-0"
    }
}

#[must_use]
pub fn getMenuLineMiddle(isOpen: bool) -> &'static str {
    if isOpen { "opacity-0" } else { "opacity-100" }
}

#[must_use]
pub fn getMenuLineBottom(isOpen: bool) -> &'static str {
    if isOpen {
        "bottom-1/2 translate-y-1/2 -rotate-45"
    } else {
        "bottom-0"
    }
}

#[must_use]
pub fn formatTime(seconds: i32, showHours: bool) -> String {
    let remaining = seconds.max(0);
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let secs = remaining % 60;
    if showHours || hours > 0 {
        format!("{hours:02}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes:02}:{secs:02}")
    }
}

#[must_use]
pub fn formatTimeWithHours(h: i32, m: i32, s: i32) -> String {
    format!("{}:{}:{}", padZero(h), padZero(m), padZero(s))
}

#[must_use]
pub fn formatTimeMinSec(m: i32, s: i32) -> String {
    format!("{}:{}", padZero(m), padZero(s))
}

#[must_use]
pub fn padZero(n: i32) -> String {
    format!("{:02}", n.max(0))
}

#[must_use]
pub fn timerAriaLabel(remainingSeconds: i32) -> String {
    if remainingSeconds <= 0 {
        return "Timer complete".to_string();
    }
    let minutes = remainingSeconds / 60;
    let seconds = remainingSeconds % 60;
    if minutes > 0 {
        format!("{minutes} minutes and {seconds} seconds remaining")
    } else {
        format!("{seconds} seconds remaining")
    }
}

#[must_use]
pub fn getTimerTextClass(remaining: i32, total: i32) -> &'static str {
    if total == 0 {
        return "";
    }
    let value = percent(remaining, total);
    if value <= 10 {
        "text-destructive animate-pulse"
    } else if value <= 25 {
        "text-warning"
    } else {
        ""
    }
}

#[must_use]
pub fn getGenerationMessageClass(state: &str) -> &'static str {
    match state {
        "generating" => "text-muted-foreground",
        "complete" => "text-success",
        "error" => "text-destructive",
        _ => "text-foreground",
    }
}

#[must_use]
pub fn getScoreClasses(score: i32, maxScore: i32) -> &'static str {
    match percent(score, maxScore) {
        80..=i32::MAX => "bg-success-muted text-success",
        60..=79 => "bg-warning-muted text-warning",
        _ => "bg-destructive-muted text-destructive",
    }
}

#[must_use]
pub fn getPercent(value: i32, max: i32) -> i32 {
    percent(value, max)
}

#[must_use]
pub fn getStatusDotColor(isOnline: bool) -> &'static str {
    getOnlineColor(isOnline)
}

#[must_use]
pub fn getMasteryColor(mastery: i32) -> &'static str {
    match mastery {
        80..=i32::MAX => "bg-success",
        60..=79 => "bg-info",
        40..=59 => "bg-warning",
        _ => "bg-muted-foreground",
    }
}

#[must_use]
pub fn getQuizResultClasses(passed: bool) -> &'static str {
    if passed {
        "border-success bg-success-muted"
    } else {
        "border-warning bg-warning-muted"
    }
}

#[must_use]
pub fn getInlineNotificationClasses(notificationType: &str) -> &'static str {
    match notificationType {
        "success" => "border-success bg-success-muted text-success",
        "error" => "border-destructive bg-destructive-muted text-destructive",
        "warning" => "border-warning bg-warning-muted text-warning",
        _ => "border-info bg-info-muted text-info",
    }
}

#[must_use]
pub fn getToastContainerClasses(position: &str) -> &'static str {
    match position {
        "top-left" => "fixed z-50 animate-in slide-in-from-top-2 top-4 left-4",
        "top-center" => "fixed z-50 animate-in slide-in-from-top-2 top-4 left-1/2 -translate-x-1/2",
        "bottom-right" => "fixed z-50 animate-in slide-in-from-top-2 bottom-4 right-4",
        "bottom-left" => "fixed z-50 animate-in slide-in-from-top-2 bottom-4 left-4",
        "bottom-center" => {
            "fixed z-50 animate-in slide-in-from-top-2 bottom-4 left-1/2 -translate-x-1/2"
        }
        _ => "fixed z-50 animate-in slide-in-from-top-2 top-4 right-4",
    }
}

#[must_use]
pub fn getRankClasses(rank: i32) -> &'static str {
    match rank {
        1 | 3 => "bg-warning-muted text-warning",
        _ => "bg-muted text-muted-foreground",
    }
}

#[must_use]
pub fn statusTypeClasses(statusType: &str) -> &'static str {
    match statusType {
        "active" | "completed" | "success" => "bg-success-muted text-success",
        "pending" | "warning" => "bg-warning-muted text-warning",
        "error" | "destructive" => "bg-destructive-muted text-destructive",
        "inactive" => "bg-muted text-muted-foreground",
        _ => "bg-muted text-muted-foreground",
    }
}

#[must_use]
pub fn metaVariantClasses(variant: &str) -> &'static str {
    match variant {
        "primary" => "text-primary font-medium",
        "success" => "text-success font-medium",
        "warning" => "text-warning font-medium",
        "destructive" => "text-destructive font-medium",
        _ => "text-muted-foreground",
    }
}

macro_rules! props_with_attributes {
    ($name:ident) => {
        impl $name {
            #[must_use]
            pub fn with_class(mut self, value: impl Into<String>) -> Self {
                self.class = value.into();
                self
            }

            #[must_use]
            pub fn with_attribute(
                mut self,
                name: impl Into<String>,
                value: impl Into<String>,
            ) -> Self {
                self.attributes.insert(name.into(), value.into());
                self
            }

            #[must_use]
            pub fn build_attributes(&self) -> Attrs {
                self.attributes.clone()
            }
        }
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnlineIndicatorProps {
    pub is_online: bool,
    pub show_pulse: bool,
    pub size: &'static str,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for OnlineIndicatorProps {
    fn default() -> Self {
        Self {
            is_online: false,
            show_pulse: true,
            size: shared::SIZE_MD,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(OnlineIndicatorProps);

impl OnlineIndicatorProps {
    #[must_use]
    pub const fn with_online(mut self, value: bool) -> Self {
        self.is_online = value;
        self
    }

    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["relative flex", self.class.as_str()])
    }

    #[must_use]
    pub fn size_classes(&self) -> &'static str {
        match self.size {
            shared::SIZE_SM => "h-2 w-2",
            shared::SIZE_LG => "h-4 w-4",
            _ => "h-3 w-3",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationBellProps {
    pub count: i32,
    pub max_count: i32,
    pub show_badge: bool,
    pub is_animating: bool,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for NotificationBellProps {
    fn default() -> Self {
        Self {
            count: 0,
            max_count: 99,
            show_badge: true,
            is_animating: false,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(NotificationBellProps);

impl NotificationBellProps {
    #[must_use]
    pub const fn with_count(mut self, value: i32) -> Self {
        self.count = value;
        self
    }

    #[must_use]
    pub const fn with_max_count(mut self, value: i32) -> Self {
        self.max_count = value;
        self
    }

    #[must_use]
    pub const fn with_animating(mut self, value: bool) -> Self {
        self.is_animating = value;
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "relative inline-flex items-center justify-center",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn display_count(&self) -> String {
        if self.count > self.max_count {
            format!("{}+", self.max_count)
        } else {
            self.count.to_string()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MobileMenuButtonProps {
    pub is_open: bool,
    pub aria_label: String,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for MobileMenuButtonProps {
    fn default() -> Self {
        Self {
            is_open: false,
            aria_label: String::new(),
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(MobileMenuButtonProps);

impl MobileMenuButtonProps {
    #[must_use]
    pub const fn with_open(mut self, value: bool) -> Self {
        self.is_open = value;
        self
    }

    #[must_use]
    pub fn with_aria_label(mut self, value: impl Into<String>) -> Self {
        self.aria_label = value.into();
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "inline-flex items-center justify-center h-10 w-10 rounded-md hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring transition-colors duration-150",
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn resolved_label(&self) -> &str {
        if self.aria_label.is_empty() {
            "Toggle menu"
        } else {
            self.aria_label.as_str()
        }
    }
}

#[must_use]
pub fn resolveMobileMenuAriaLabel(props: &MobileMenuButtonProps) -> String {
    props.resolved_label().to_string()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnimatedProgressProps {
    pub value: i32,
    pub max: i32,
    pub show_label: bool,
    pub animate_on_load: bool,
    pub variant: &'static str,
    pub size: &'static str,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for AnimatedProgressProps {
    fn default() -> Self {
        Self {
            value: 0,
            max: 100,
            show_label: false,
            animate_on_load: true,
            variant: shared::VARIANT_PRIMARY,
            size: shared::SIZE_MD,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(AnimatedProgressProps);

impl AnimatedProgressProps {
    #[must_use]
    pub const fn with_value(mut self, value: i32) -> Self {
        self.value = value;
        self
    }

    #[must_use]
    pub const fn with_max(mut self, max: i32) -> Self {
        self.max = max;
        self
    }

    #[must_use]
    pub const fn with_show_label(mut self, show: bool) -> Self {
        self.show_label = show;
        self
    }

    #[must_use]
    pub const fn with_variant(mut self, variant: &'static str) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub const fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn progress_percent(&self) -> i32 {
        clampedPercent(self.value, self.max)
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "w-full bg-muted rounded-full overflow-hidden",
            shared::progress_size_classes(self.size),
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn bar_classes(&self) -> String {
        shared::build_classes(&[
            "h-full rounded-full",
            shared::progress_variant_classes(self.variant),
            if self.animate_on_load {
                "transition-all duration-1000 ease-out"
            } else {
                ""
            },
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpinnerProps {
    pub size: &'static str,
    pub variant: &'static str,
    pub label: String,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for SpinnerProps {
    fn default() -> Self {
        Self {
            size: shared::SIZE_MD,
            variant: shared::VARIANT_PRIMARY,
            label: String::new(),
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(SpinnerProps);

impl SpinnerProps {
    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }

    #[must_use]
    pub const fn with_variant(mut self, value: &'static str) -> Self {
        self.variant = value;
        self
    }

    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
        self
    }

    #[must_use]
    pub fn resolved_label(&self) -> &str {
        if self.label.is_empty() {
            shared::default_strings().loading_text
        } else {
            self.label.as_str()
        }
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let size = match self.size {
            shared::SIZE_XS => "h-3.5 w-3.5",
            shared::SIZE_SM => "h-4 w-4",
            shared::SIZE_LG => "h-8 w-8",
            shared::SIZE_XL => "h-12 w-12",
            _ => "h-6 w-6",
        };
        shared::build_classes(&[
            "animate-spin rounded-full border-2 border-current border-t-transparent",
            shared::text_color_classes(self.variant),
            size,
            self.class.as_str(),
        ])
    }
}

#[must_use]
pub fn resolveSpinnerLabel(label: &str) -> String {
    if label.is_empty() {
        shared::default_strings().loading_text.to_string()
    } else {
        label.to_string()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SVGSpinnerProps {
    pub size: &'static str,
    pub label: String,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for SVGSpinnerProps {
    fn default() -> Self {
        Self {
            size: shared::SIZE_MD,
            label: String::new(),
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(SVGSpinnerProps);

impl SVGSpinnerProps {
    #[must_use]
    pub const fn with_size(mut self, value: &'static str) -> Self {
        self.size = value;
        self
    }

    #[must_use]
    pub fn with_label(mut self, value: impl Into<String>) -> Self {
        self.label = value.into();
        self
    }

    #[must_use]
    pub fn resolved_label(&self) -> &str {
        if self.label.is_empty() {
            shared::default_strings().loading_text
        } else {
            self.label.as_str()
        }
    }

    #[must_use]
    pub fn size_classes(&self) -> &'static str {
        match self.size {
            shared::SIZE_XS => "w-3 h-3",
            shared::SIZE_SM => "w-3.5 h-3.5",
            shared::SIZE_LG => "w-7 h-7",
            shared::SIZE_XL => "w-10 h-10",
            _ => "w-5 h-5",
        }
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["animate-spin", self.size_classes(), self.class.as_str()])
    }
}

#[must_use]
pub fn resolveSVGSpinnerLabel(label: &str) -> String {
    if label.is_empty() {
        shared::default_strings().loading_text.to_string()
    } else {
        label.to_string()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimerProps {
    pub duration_seconds: i32,
    pub remaining_seconds: i32,
    pub is_running: bool,
    pub show_hours: bool,
    pub variant: &'static str,
    pub size: &'static str,
    pub state_var: String,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for TimerProps {
    fn default() -> Self {
        Self {
            duration_seconds: 0,
            remaining_seconds: 0,
            is_running: false,
            show_hours: false,
            variant: shared::VARIANT_DEFAULT,
            size: shared::SIZE_MD,
            state_var: "timerState".to_string(),
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(TimerProps);

impl TimerProps {
    #[must_use]
    pub const fn with_duration(mut self, value: i32) -> Self {
        self.duration_seconds = value;
        self.remaining_seconds = value;
        self
    }

    #[must_use]
    pub const fn with_remaining(mut self, value: i32) -> Self {
        self.remaining_seconds = value;
        self
    }

    #[must_use]
    pub const fn with_running(mut self, value: bool) -> Self {
        self.is_running = value;
        self
    }

    #[must_use]
    pub const fn with_show_hours(mut self, value: bool) -> Self {
        self.show_hours = value;
        self
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        let size = match self.size {
            shared::SIZE_SM => "text-sm",
            shared::SIZE_LG => "text-lg",
            shared::SIZE_XL => "text-2xl",
            _ => "text-base",
        };
        shared::build_classes(&[
            "inline-flex items-center justify-center font-mono tabular-nums",
            size,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn format_time(&self) -> String {
        formatTime(self.remaining_seconds, self.show_hours)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationStateIndicatorProps {
    pub state: String,
    pub message: String,
    pub show_spinner: bool,
    pub variant: &'static str,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for GenerationStateIndicatorProps {
    fn default() -> Self {
        Self {
            state: "idle".to_string(),
            message: String::new(),
            show_spinner: true,
            variant: shared::VARIANT_DEFAULT,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(GenerationStateIndicatorProps);

impl GenerationStateIndicatorProps {
    #[must_use]
    pub fn with_state(mut self, value: impl Into<String>) -> Self {
        self.state = value.into();
        self
    }

    #[must_use]
    pub fn with_message(mut self, value: impl Into<String>) -> Self {
        self.message = value.into();
        self
    }

    #[must_use]
    pub const fn is_generating(&self) -> bool {
        matches!(self.state.as_bytes(), b"generating")
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self.state.as_bytes(), b"complete")
    }

    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self.state.as_bytes(), b"error")
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["inline-flex items-center gap-2", self.class.as_str()])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessingProgressProps {
    pub message: String,
    pub percent: i32,
    pub variant: &'static str,
    pub show_label: bool,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for ProcessingProgressProps {
    fn default() -> Self {
        Self {
            message: String::new(),
            percent: 0,
            variant: shared::VARIANT_PRIMARY,
            show_label: true,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(ProcessingProgressProps);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatFragmentProps {
    pub value: String,
    pub label: String,
    pub icon: String,
    pub value_size: &'static str,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for StatFragmentProps {
    fn default() -> Self {
        Self {
            value: String::new(),
            label: String::new(),
            icon: String::new(),
            value_size: shared::SIZE_XL,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(StatFragmentProps);

impl StatFragmentProps {
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["flex flex-col items-center gap-1", self.class.as_str()])
    }

    #[must_use]
    pub fn value_classes(&self) -> &'static str {
        match self.value_size {
            shared::SIZE_SM => "text-sm font-semibold text-foreground",
            shared::SIZE_MD => "text-lg font-bold text-foreground",
            shared::SIZE_LG => "text-xl font-bold text-foreground",
            _ => "text-2xl font-bold text-foreground",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationMessageProps {
    pub field: String,
    pub message: String,
    pub is_error: bool,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for ValidationMessageProps {
    fn default() -> Self {
        Self {
            field: String::new(),
            message: String::new(),
            is_error: true,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(ValidationMessageProps);

impl ValidationMessageProps {
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "text-sm mt-1",
            if self.is_error {
                "text-destructive"
            } else {
                "text-success"
            },
            self.class.as_str(),
        ])
    }
}

#[must_use]
pub fn getValidationRole(isError: bool) -> &'static str {
    if isError { "alert" } else { "status" }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToastNotificationProps {
    pub message: String,
    pub variant: &'static str,
    pub dismissible: bool,
    pub duration: i32,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for ToastNotificationProps {
    fn default() -> Self {
        Self {
            message: String::new(),
            variant: shared::VARIANT_INFO,
            dismissible: true,
            duration: 5000,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(ToastNotificationProps);

impl ToastNotificationProps {
    #[must_use]
    pub fn build_classes(&self) -> String {
        let variant = match self.variant {
            shared::VARIANT_SUCCESS => "border-success bg-success-muted",
            shared::VARIANT_DESTRUCTIVE => "border-destructive bg-destructive-muted",
            shared::VARIANT_WARNING => "border-warning bg-warning-muted",
            _ => "border-info bg-info-muted",
        };
        shared::build_classes(&[
            "flex items-center gap-3 rounded-lg border p-4 shadow-lg",
            variant,
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn text_classes(&self) -> &'static str {
        match self.variant {
            shared::VARIANT_SUCCESS => "text-success-foreground",
            shared::VARIANT_DESTRUCTIVE => "text-destructive-foreground",
            shared::VARIANT_WARNING => "text-warning-foreground",
            _ => "text-info-foreground",
        }
    }

    #[must_use]
    pub fn icon_classes(&self) -> &'static str {
        match self.variant {
            shared::VARIANT_SUCCESS => "text-success",
            shared::VARIANT_DESTRUCTIVE => "text-destructive",
            shared::VARIANT_WARNING => "text-warning",
            _ => "text-info",
        }
    }

    #[must_use]
    pub fn icon(&self) -> &'static str {
        match self.variant {
            shared::VARIANT_SUCCESS => "✓",
            shared::VARIANT_DESTRUCTIVE => "✕",
            shared::VARIANT_WARNING => "⚠",
            _ => "ℹ",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ActivityItemProps {
    pub username: String,
    pub action: String,
    pub target: String,
    pub timestamp: String,
    pub icon: String,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(ActivityItemProps);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CountBadgeProps {
    pub count: i32,
    pub label: String,
    pub variant: &'static str,
    pub show_zero: bool,
    pub max_count: i32,
    pub pulsing: bool,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for CountBadgeProps {
    fn default() -> Self {
        Self {
            count: 0,
            label: String::new(),
            variant: shared::VARIANT_DEFAULT,
            show_zero: false,
            max_count: 99,
            pulsing: false,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(CountBadgeProps);

impl CountBadgeProps {
    #[must_use]
    pub const fn should_show(&self) -> bool {
        self.count > 0 || self.show_zero
    }

    #[must_use]
    pub fn display_count(&self) -> String {
        if self.count > self.max_count {
            format!("{}+", self.max_count)
        } else {
            self.count.to_string()
        }
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "inline-flex items-center justify-center min-w-[1.25rem] h-5 px-1.5 text-xs font-medium rounded-full",
            shared::badge_variant_classes(self.variant),
            if self.pulsing && self.count > 0 {
                "animate-pulse"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingCountBadgeProps {
    pub count_badge_props: CountBadgeProps,
}

#[must_use]
pub fn NewPendingCountBadge(count: i32, label: &str) -> PendingCountBadgeProps {
    PendingCountBadgeProps {
        count_badge_props: CountBadgeProps {
            count,
            label: label.to_string(),
            variant: shared::VARIANT_WARNING,
            show_zero: false,
            max_count: 99,
            pulsing: count > 0,
            ..Default::default()
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedCountBadgeProps {
    pub count_badge_props: CountBadgeProps,
}

#[must_use]
pub fn NewCompletedCountBadge(count: i32, label: &str) -> CompletedCountBadgeProps {
    CompletedCountBadgeProps {
        count_badge_props: CountBadgeProps {
            count,
            label: label.to_string(),
            variant: shared::VARIANT_SUCCESS,
            show_zero: true,
            max_count: 999,
            pulsing: false,
            ..Default::default()
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnreadCountBadgeProps {
    pub count_badge_props: CountBadgeProps,
}

#[must_use]
pub fn NewUnreadCountBadge(count: i32) -> UnreadCountBadgeProps {
    UnreadCountBadgeProps {
        count_badge_props: CountBadgeProps {
            count,
            variant: shared::VARIANT_DESTRUCTIVE,
            show_zero: false,
            max_count: 99,
            pulsing: count > 0,
            ..Default::default()
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormLevelValidationProps {
    pub title: String,
    pub messages: Vec<String>,
    pub is_error: bool,
    pub dismissible: bool,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for FormLevelValidationProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            messages: Vec::new(),
            is_error: true,
            dismissible: true,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(FormLevelValidationProps);

impl FormLevelValidationProps {
    #[must_use]
    pub fn build_classes(&self) -> String {
        let variant = if self.is_error {
            "border-destructive bg-destructive-muted"
        } else {
            "border-success bg-success-muted"
        };
        shared::build_classes(&["rounded-lg border p-4", variant, self.class.as_str()])
    }

    #[must_use]
    pub const fn role(&self) -> &'static str {
        if self.is_error { "alert" } else { "status" }
    }

    #[must_use]
    pub const fn icon(&self) -> &'static str {
        if self.is_error { "✕" } else { "✓" }
    }

    #[must_use]
    pub const fn title_classes(&self) -> &'static str {
        if self.is_error {
            "font-semibold text-destructive"
        } else {
            "font-semibold text-success"
        }
    }

    #[must_use]
    pub const fn list_classes(&self) -> &'static str {
        if self.is_error {
            "text-destructive/90"
        } else {
            "text-success/90"
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormLevelErrorProps {
    pub form_level_validation_props: FormLevelValidationProps,
}

#[must_use]
pub fn NewFormLevelError(title: &str, messages: Vec<String>) -> FormLevelErrorProps {
    FormLevelErrorProps {
        form_level_validation_props: FormLevelValidationProps {
            title: title.to_string(),
            messages,
            is_error: true,
            dismissible: true,
            ..Default::default()
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormLevelSuccessProps {
    pub form_level_validation_props: FormLevelValidationProps,
}

#[must_use]
pub fn NewFormLevelSuccess(title: &str, messages: Vec<String>) -> FormLevelSuccessProps {
    FormLevelSuccessProps {
        form_level_validation_props: FormLevelValidationProps {
            title: title.to_string(),
            messages,
            is_error: false,
            dismissible: true,
            ..Default::default()
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemProgressProps {
    pub title: String,
    pub subtitle: String,
    pub status: String,
    pub percent: i32,
    pub icon: String,
    pub badge: String,
    pub badge_variant: &'static str,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for ItemProgressProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            subtitle: String::new(),
            status: String::new(),
            percent: 0,
            icon: String::new(),
            badge: String::new(),
            badge_variant: shared::VARIANT_DEFAULT,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(ItemProgressProps);

impl ItemProgressProps {
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["p-3 rounded-lg border bg-card", self.class.as_str()])
    }

    #[must_use]
    pub fn is_processing(&self) -> bool {
        matches!(
            self.status.as_str(),
            "processing" | "generating" | "uploading"
        )
    }

    #[must_use]
    pub fn status_icon(&self) -> &'static str {
        match self.status.as_str() {
            "complete" | "completed" => "✓",
            "error" | "failed" => "✕",
            "processing" | "generating" | "uploading" => "…",
            _ => "•",
        }
    }

    #[must_use]
    pub fn status_color(&self) -> &'static str {
        match self.status.as_str() {
            "complete" | "completed" => "text-success",
            "error" | "failed" => "text-destructive",
            "processing" | "generating" | "uploading" => "text-primary",
            _ => "text-muted-foreground",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationProgressProps {
    pub title: String,
    pub subtitle: String,
    pub status: String,
    pub message: String,
    pub items_count: i32,
    pub items_label: String,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for GenerationProgressProps {
    fn default() -> Self {
        Self {
            title: String::new(),
            subtitle: String::new(),
            status: "generating".to_string(),
            message: String::new(),
            items_count: 0,
            items_label: "items".to_string(),
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(GenerationProgressProps);

impl GenerationProgressProps {
    #[must_use]
    pub fn is_generating(&self) -> bool {
        self.status == "generating"
    }

    #[must_use]
    pub fn status_icon(&self) -> &'static str {
        match self.status.as_str() {
            "complete" => "✓",
            "error" => "✕",
            _ => "…",
        }
    }

    #[must_use]
    pub fn text_classes(&self) -> &'static str {
        match self.status.as_str() {
            "complete" => "text-success",
            "error" => "text-destructive",
            _ => "text-foreground",
        }
    }

    #[must_use]
    pub fn muted_text_classes(&self) -> &'static str {
        match self.status.as_str() {
            "complete" => "text-success/80",
            "error" => "text-destructive/80",
            _ => "text-muted-foreground",
        }
    }

    #[must_use]
    pub fn format_items_count(&self) -> String {
        format!("{} {}", self.items_count, self.items_label)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct StreamingContentProps {
    pub icon: String,
    pub content: String,
    pub is_complete: bool,
    pub show_cursor: bool,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(StreamingContentProps);

impl StreamingContentProps {
    #[must_use]
    pub const fn should_show_cursor(&self) -> bool {
        self.show_cursor && !self.is_complete
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ProgressCounterProps {
    pub current: i32,
    pub total: i32,
    pub label: String,
    pub show_bar: bool,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(ProgressCounterProps);

impl ProgressCounterProps {
    #[must_use]
    pub fn progress_percent(&self) -> i32 {
        clampedPercent(self.current, self.total)
    }

    #[must_use]
    pub fn format_progress(&self) -> String {
        if self.label.is_empty() {
            format!("{} / {}", self.current, self.total)
        } else {
            format!("{} / {} {}", self.current, self.total, self.label)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ChatBubbleProps {
    pub content: String,
    pub is_outgoing: bool,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(ChatBubbleProps);

impl ChatBubbleProps {
    #[must_use]
    pub fn wrapper_classes(&self) -> &'static str {
        if self.is_outgoing {
            "flex justify-end"
        } else {
            "flex justify-start"
        }
    }

    #[must_use]
    pub fn bubble_classes(&self) -> String {
        shared::build_classes(&[
            "max-w-[75%] rounded-lg px-3 py-2",
            if self.is_outgoing {
                "bg-primary text-primary-foreground"
            } else {
                "bg-muted text-foreground"
            },
            self.class.as_str(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct StatCardProps {
    pub title: String,
    pub value: String,
    pub subtitle: String,
    pub icon: String,
    pub trend: String,
    pub trend_value: String,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(StatCardProps);

impl StatCardProps {
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["rounded-lg border bg-card p-6", self.class.as_str()])
    }

    #[must_use]
    pub fn trend_color(&self) -> &'static str {
        match self.trend.as_str() {
            "up" | "positive" => "text-success",
            "down" | "negative" => "text-destructive",
            _ => "text-muted-foreground",
        }
    }

    #[must_use]
    pub fn trend_icon(&self) -> &'static str {
        match self.trend.as_str() {
            "up" | "positive" => "↑",
            "down" | "negative" => "↓",
            _ => "→",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaginationProps {
    pub current_page: i32,
    pub total_pages: i32,
    pub base_url: String,
    pub show_prev_next: bool,
    pub show_numbers: bool,
    pub max_visible: i32,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for PaginationProps {
    fn default() -> Self {
        Self {
            current_page: 1,
            total_pages: 1,
            base_url: String::new(),
            show_prev_next: true,
            show_numbers: true,
            max_visible: 5,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(PaginationProps);

impl PaginationProps {
    #[must_use]
    pub const fn has_prev(&self) -> bool {
        self.current_page > 1
    }

    #[must_use]
    pub const fn has_next(&self) -> bool {
        self.current_page < self.total_pages
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&["flex items-center gap-1", self.class.as_str()])
    }

    #[must_use]
    pub fn get_page_url(&self, page: i32) -> String {
        if self.base_url.contains('?') {
            format!("{}&page={}", self.base_url, page)
        } else {
            format!("{}?page={}", self.base_url, page)
        }
    }

    #[must_use]
    pub fn visible_pages(&self) -> Vec<i32> {
        let max_visible = self.max_visible.max(1);
        let half = max_visible / 2;
        let mut start = (self.current_page - half).max(1);
        let mut end = (start + max_visible - 1).min(self.total_pages);
        if end - start + 1 < max_visible {
            start = (end - max_visible + 1).max(1);
            end = (start + max_visible - 1).min(self.total_pages);
        }
        (start..=end).collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimpleStatProps {
    pub value: String,
    pub label: String,
    pub align: String,
    pub size: &'static str,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for SimpleStatProps {
    fn default() -> Self {
        Self {
            value: String::new(),
            label: String::new(),
            align: "center".to_string(),
            size: shared::SIZE_XL,
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(SimpleStatProps);

impl SimpleStatProps {
    #[must_use]
    pub fn build_classes(&self) -> String {
        let align = match self.align.as_str() {
            "start" | "left" => "items-start text-left",
            "end" | "right" => "items-end text-right",
            _ => "items-center text-center",
        };
        shared::build_classes(&["flex flex-col gap-1", align, self.class.as_str()])
    }

    #[must_use]
    pub fn value_classes(&self) -> &'static str {
        match self.size {
            shared::SIZE_SM => "text-sm font-semibold",
            shared::SIZE_MD => "text-lg font-bold",
            shared::SIZE_LG => "text-xl font-bold",
            _ => "text-2xl font-bold",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct IconStatCardProps {
    pub label: String,
    pub value: String,
    pub icon: String,
    pub style: String,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(IconStatCardProps);

impl IconStatCardProps {
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "rounded-lg border bg-card p-6",
            if self.style == "neomorph" {
                "neomorph-flat"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct UserListItemProps {
    pub name: String,
    pub subtitle: String,
    pub href: String,
    pub avatar_src: String,
    pub show_online: bool,
    pub is_online: bool,
    pub stats: Vec<String>,
    pub value: String,
    pub value_label: String,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(UserListItemProps);

impl UserListItemProps {
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "flex items-center justify-between gap-4 rounded-lg border bg-card p-4 transition-colors",
            if self.href.is_empty() {
                ""
            } else {
                "hover:bg-muted/50"
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn initials(&self) -> String {
        let mut result = String::new();
        for ch in self
            .name
            .split_whitespace()
            .take(2)
            .filter_map(|part| part.chars().next())
        {
            result.push(ch.to_ascii_uppercase());
        }
        if result.is_empty() {
            "?".to_string()
        } else {
            result
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct RankingCardProps {
    pub rank: i32,
    pub name: String,
    pub subtitle: String,
    pub value: i32,
    pub value_label: String,
    pub secondary: String,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(RankingCardProps);

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct SectionCardProps {
    pub title: String,
    pub subtitle: String,
    pub icon: String,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(SectionCardProps);

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct MasteryDisplayProps {
    pub percent: i32,
    pub label: String,
    pub show_bar: bool,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(MasteryDisplayProps);

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ActivityRowStat {
    pub value: String,
    pub label: String,
    pub variant: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ActivityRowProps {
    pub date: String,
    pub stats: Vec<ActivityRowStat>,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(ActivityRowProps);

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct TranscriptMessageProps {
    pub role: String,
    pub role_label: String,
    pub content: String,
    pub html_content: String,
    pub timestamp: String,
    pub timestamp_iso: String,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(TranscriptMessageProps);

impl TranscriptMessageProps {
    #[must_use]
    pub fn label(&self) -> &str {
        if !self.role_label.is_empty() {
            return self.role_label.as_str();
        }
        match self.role.as_str() {
            "assistant" => "Assistant",
            "user" => "User",
            "system" => "System",
            value => value,
        }
    }

    #[must_use]
    pub fn label_class(&self) -> &'static str {
        match self.role.as_str() {
            "assistant" => "text-primary",
            "user" => "text-success",
            "system" => "text-warning",
            _ => "text-muted-foreground",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct IconStat {
    pub icon: String,
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ListItemLinkCardProps {
    pub href: String,
    pub title: String,
    pub status: String,
    pub status_class: String,
    pub timestamp: String,
    pub created_at: String,
    pub show_arrow: bool,
    pub stats: Vec<IconStat>,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(ListItemLinkCardProps);

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct StatIconRowProps {
    pub stats: Vec<IconStat>,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(StatIconRowProps);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusBadgeProps {
    pub status: String,
    pub status_type: String,
    pub class: String,
    pub attributes: Attrs,
}

impl Default for StatusBadgeProps {
    fn default() -> Self {
        Self {
            status: String::new(),
            status_type: "inactive".to_string(),
            class: String::new(),
            attributes: Attrs::new(),
        }
    }
}

props_with_attributes!(StatusBadgeProps);

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ItemInfoMeta {
    pub icon: String,
    pub label: String,
    pub value: String,
    pub variant: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ItemInfoCardProps {
    pub icon: String,
    pub title: String,
    pub description: String,
    pub badge: String,
    pub badge_class: String,
    pub is_disabled: bool,
    pub metadata: Vec<ItemInfoMeta>,
    pub class: String,
    pub attributes: Attrs,
}

props_with_attributes!(ItemInfoCardProps);

impl ItemInfoCardProps {
    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[
            "rounded-lg border bg-card",
            if self.is_disabled {
                "opacity-60 grayscale"
            } else {
                ""
            },
            self.class.as_str(),
        ])
    }

    #[must_use]
    pub fn badge_classes(&self) -> String {
        shared::build_classes(&[
            "rounded-full px-2 py-0.5 text-xs font-medium",
            if self.badge_class.is_empty() {
                "bg-muted text-muted-foreground"
            } else {
                self.badge_class.as_str()
            },
        ])
    }
}

#[must_use]
pub fn OnlineIndicator(props: &OnlineIndicatorProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        span class=(props.build_classes()) aria-label=(getOnlineLabel(props.is_online)) data-testid=[attr(&attrs, "data-testid")] {
            @if props.is_online && props.show_pulse {
                span class=(shared::build_classes(&["animate-ping absolute inline-flex rounded-full opacity-75", props.size_classes(), "bg-success"])) {}
            }
            span class=(shared::build_classes(&["relative inline-flex rounded-full", props.size_classes(), getOnlineColor(props.is_online)])) {}
        }
    }
}

#[must_use]
pub fn NotificationBell(props: &NotificationBellProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        button type="button" class=(props.build_classes()) aria-label=(getNotificationLabel(props.count)) data-testid=[attr(&attrs, "data-testid")] {
            svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class=(getBellClass(props.is_animating)) {
                path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" {}
                path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" {}
            }
            @if props.show_badge && props.count > 0 {
                span class="absolute -top-1 -right-1 flex items-center justify-center min-w-[18px] h-[18px] px-1 text-[10px] font-bold bg-destructive text-destructive-foreground rounded-full" {
                    (props.display_count())
                }
            }
        }
    }
}

#[must_use]
pub fn MobileMenuButton(props: &MobileMenuButtonProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        button type="button" class=(props.build_classes()) aria-label=(props.resolved_label()) aria-expanded=(boolStr(props.is_open)) data-testid=[attr(&attrs, "data-testid")] {
            div class="relative w-5 h-5" {
                span class=(shared::build_classes(&["absolute block h-0.5 w-5 bg-current transform transition-all duration-300", getMenuLineTop(props.is_open)])) {}
                span class=(shared::build_classes(&["absolute block h-0.5 w-5 bg-current top-1/2 -translate-y-1/2 transition-all duration-300", getMenuLineMiddle(props.is_open)])) {}
                span class=(shared::build_classes(&["absolute block h-0.5 w-5 bg-current transform transition-all duration-300", getMenuLineBottom(props.is_open)])) {}
            }
        }
    }
}

#[must_use]
pub fn AnimatedProgress(props: &AnimatedProgressProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class="w-full" {
            @if props.show_label {
                div class="flex justify-between text-sm mb-1" {
                    span class="text-muted-foreground" { "Progress" }
                    span class="font-medium" { (props.progress_percent()) "%" }
                }
            }
            div class=(props.build_classes()) role="progressbar" aria-valuenow=(props.value) aria-valuemin="0" aria-valuemax=(props.max) data-testid=[attr(&attrs, "data-testid")] {
                div class=(props.bar_classes()) style=(format!("width: {}%", props.progress_percent())) {}
            }
        }
    }
}

#[must_use]
pub fn SimpleOnlineIndicator(isOnline: bool, class: &str) -> Markup {
    OnlineIndicator(
        &OnlineIndicatorProps::default()
            .with_online(isOnline)
            .with_class(class),
    )
}

#[must_use]
pub fn SimpleNotificationBell(count: i32, class: &str) -> Markup {
    NotificationBell(
        &NotificationBellProps::default()
            .with_count(count)
            .with_class(class),
    )
}

#[must_use]
pub fn SimpleMobileMenuButton(isOpen: bool, class: &str) -> Markup {
    MobileMenuButton(
        &MobileMenuButtonProps::default()
            .with_open(isOpen)
            .with_class(class),
    )
}

#[must_use]
pub fn SimpleAnimatedProgress(value: i32, max: i32, class: &str) -> Markup {
    AnimatedProgress(
        &AnimatedProgressProps::default()
            .with_value(value)
            .with_max(max)
            .with_class(class),
    )
}

#[must_use]
pub fn Spinner(props: &SpinnerProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) role="status" aria-label=(props.resolved_label()) data-testid=[attr(&attrs, "data-testid")] {
            span class="sr-only" { (props.resolved_label()) }
        }
    }
}

#[must_use]
pub fn SimpleSpinner(class: &str) -> Markup {
    Spinner(&SpinnerProps::default().with_class(class))
}

#[must_use]
pub fn SVGSpinner(props: &SVGSpinnerProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) viewBox="0 0 24 24" fill="none" role="status" aria-label=(props.resolved_label()) data-testid=[attr(&attrs, "data-testid")] {
            circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" {}
            path class="opacity-100" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" {}
            title { (props.resolved_label()) }
        }
    }
}

#[must_use]
pub fn SimpleSVGSpinner(size: &str, class: &str) -> Markup {
    let size = match size {
        shared::SIZE_XS => shared::SIZE_XS,
        shared::SIZE_SM => shared::SIZE_SM,
        shared::SIZE_LG => shared::SIZE_LG,
        shared::SIZE_XL => shared::SIZE_XL,
        _ => shared::SIZE_MD,
    };
    SVGSpinner(
        &SVGSpinnerProps::default()
            .with_size(size)
            .with_label(shared::default_strings().loading_text)
            .with_class(class),
    )
}

#[must_use]
pub fn Timer(props: &TimerProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) role="timer" aria-live="polite" aria-label=(timerAriaLabel(props.remaining_seconds)) data-testid=[attr(&attrs, "data-testid")] {
            span class=(getTimerTextClass(props.remaining_seconds, props.duration_seconds)) { (props.format_time()) }
            @if props.is_running {
                span class="ml-2 text-xs text-muted-foreground" { "Running" }
            }
        }
    }
}

#[must_use]
pub fn SimpleTimer(durationSeconds: i32, remainingSeconds: i32, class: &str) -> Markup {
    Timer(
        &TimerProps::default()
            .with_duration(durationSeconds)
            .with_remaining(remainingSeconds)
            .with_class(class),
    )
}

#[must_use]
pub fn SessionTimer(durationSeconds: i32, remainingSeconds: i32, isRunning: bool) -> Markup {
    Timer(
        &TimerProps::default()
            .with_duration(durationSeconds)
            .with_remaining(remainingSeconds)
            .with_running(isRunning)
            .with_show_hours(durationSeconds >= 3600),
    )
}

#[must_use]
pub fn QuizTimer(durationSeconds: i32, remainingSeconds: i32, isRunning: bool) -> Markup {
    Timer(
        &TimerProps::default()
            .with_duration(durationSeconds)
            .with_remaining(remainingSeconds)
            .with_running(isRunning)
            .with_class("font-bold"),
    )
}

#[must_use]
pub fn GenerationStateIndicator(props: &GenerationStateIndicatorProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) role="status" aria-live="polite" data-testid=[attr(&attrs, "data-testid")] {
            @if props.is_generating() && props.show_spinner {
                (Spinner(&SpinnerProps::default().with_label("Generating")))
            }
            @if props.is_complete() {
                span class="text-success" data-icon="check" { "✓" }
            }
            @if props.is_error() {
                span class="text-destructive" data-icon="x-circle" { "✕" }
            }
            @if !props.message.is_empty() {
                span class=(getGenerationMessageClass(&props.state)) { (&props.message) }
            }
        }
    }
}

#[must_use]
pub fn SimpleGenerationStateIndicator(state: &str, message: &str) -> Markup {
    GenerationStateIndicator(
        &GenerationStateIndicatorProps::default()
            .with_state(state)
            .with_message(message),
    )
}

#[must_use]
pub fn ProcessingProgress(props: &ProcessingProgressProps) -> Markup {
    let attrs = props.build_attributes();
    let progress = props.percent.clamp(0, 100);
    html! {
        div class=(shared::build_classes(&["space-y-2", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            @if props.show_label {
                div class="flex items-center justify-between text-sm" {
                    span class="text-muted-foreground" { (&props.message) }
                    span class="font-medium" { (progress) "%" }
                }
            }
            div class="h-2 bg-muted rounded-full overflow-hidden" {
                div class="h-full bg-primary rounded-full transition-all duration-300" style=(format!("width: {progress}%")) {}
            }
        }
    }
}

#[must_use]
pub fn SimpleProcessingProgress(message: &str, percent: i32) -> Markup {
    ProcessingProgress(&ProcessingProgressProps {
        message: message.to_string(),
        percent,
        ..Default::default()
    })
}

#[must_use]
pub fn ProcessingSpinnerMessage(message: &str, class: &str) -> Markup {
    html! {
        div class=(shared::build_classes(&["flex items-center gap-3 p-4 bg-muted/50 rounded-lg", class])) {
            (SimpleSpinner(""))
            span class="text-sm text-muted-foreground" { (message) }
        }
    }
}

#[must_use]
pub fn ProcessingComplete(message: &str, class: &str) -> Markup {
    html! {
        div class=(shared::build_classes(&["flex items-center gap-3 p-4 bg-success-muted rounded-lg border border-success", class])) {
            span class="text-success" data-icon="check-circle" { "✓" }
            span class="text-sm font-medium text-success-foreground" { (message) }
        }
    }
}

#[must_use]
pub fn iconOrSymbol(value: &str, class: &str, size: &str) -> Markup {
    if value.is_empty() {
        html! {}
    } else {
        html! { span class=[optional(class)] data-icon=(value) data-size=(size) { (value) } }
    }
}

#[must_use]
pub fn ProcessingError(message: &str, class: &str) -> Markup {
    html! {
        div class=(shared::build_classes(&["flex items-center gap-3 p-4 bg-destructive-muted rounded-lg border border-destructive", class])) {
            span class="text-destructive" data-icon="x-circle" { "✕" }
            span class="text-sm font-medium text-destructive-foreground" { (message) }
        }
    }
}

#[must_use]
pub fn StatFragment(props: &StatFragmentProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            @if !props.icon.is_empty() {
                (iconOrSymbol(&props.icon, "text-xl", shared::SIZE_XL))
            }
            span class=(props.value_classes()) { (&props.value) }
            @if !props.label.is_empty() {
                span class="text-sm text-muted-foreground" { (&props.label) }
            }
        }
    }
}

#[must_use]
pub fn SimpleStatFragment(value: &str, label: &str) -> Markup {
    StatFragment(&StatFragmentProps {
        value: value.to_string(),
        label: label.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn StatFragmentWithIcon(value: &str, label: &str, icon: &str) -> Markup {
    StatFragment(&StatFragmentProps {
        value: value.to_string(),
        label: label.to_string(),
        icon: icon.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn ValidationMessage(props: &ValidationMessageProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) id=(format!("{}-error", props.field)) data-testid=[attr(&attrs, "data-testid")] {
            (&props.message)
        }
    }
}

#[must_use]
pub fn ValidationError(field: &str, message: &str) -> Markup {
    ValidationMessage(&ValidationMessageProps {
        field: field.to_string(),
        message: message.to_string(),
        is_error: true,
        ..Default::default()
    })
}

#[must_use]
pub fn ValidationSuccess(field: &str) -> Markup {
    ValidationMessage(&ValidationMessageProps {
        field: field.to_string(),
        message: "✓".to_string(),
        is_error: false,
        ..Default::default()
    })
}

#[must_use]
pub fn ToastNotification(props: &ToastNotificationProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] data-duration=(props.duration) {
            span class=(props.icon_classes()) { (props.icon()) }
            p class=(shared::build_classes(&["text-sm font-medium flex-1", props.text_classes()])) { (&props.message) }
            @if props.dismissible {
                button type="button" class=(shared::build_classes(&["opacity-70 hover:opacity-100", props.icon_classes()])) data-on:click="evt.currentTarget.parentElement.remove()" { "×" }
            }
        }
    }
}

#[must_use]
pub fn ToastSuccess(message: &str) -> Markup {
    ToastNotification(&ToastNotificationProps {
        message: message.to_string(),
        variant: shared::VARIANT_SUCCESS,
        ..Default::default()
    })
}

#[must_use]
pub fn ToastError(message: &str) -> Markup {
    ToastNotification(&ToastNotificationProps {
        message: message.to_string(),
        variant: shared::VARIANT_DESTRUCTIVE,
        ..Default::default()
    })
}

#[must_use]
pub fn ToastWarning(message: &str) -> Markup {
    ToastNotification(&ToastNotificationProps {
        message: message.to_string(),
        variant: shared::VARIANT_WARNING,
        ..Default::default()
    })
}

#[must_use]
pub fn ToastInfo(message: &str) -> Markup {
    ToastNotification(&ToastNotificationProps {
        message: message.to_string(),
        variant: shared::VARIANT_INFO,
        ..Default::default()
    })
}

#[must_use]
pub fn ActivityItem(props: &ActivityItemProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["p-3 bg-muted/50 rounded-lg", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-center gap-2" {
                @if !props.icon.is_empty() {
                    (iconOrSymbol(&props.icon, "text-sm", shared::SIZE_SM))
                }
                span class="font-medium text-sm" { (&props.username) }
                span class="text-sm text-muted-foreground" { (&props.action) }
                @if !props.target.is_empty() {
                    span class="font-medium text-sm" { (&props.target) }
                }
            }
            span class="text-xs text-muted-foreground mt-1" { (&props.timestamp) }
        }
    }
}

#[must_use]
pub fn SimpleActivityItem(username: &str, action: &str) -> Markup {
    ActivityItem(&ActivityItemProps {
        username: username.to_string(),
        action: action.to_string(),
        timestamp: "just now".to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn TypingIndicator(class: &str) -> Markup {
    html! {
        div class=(shared::build_classes(&["flex items-center gap-2 p-3 bg-muted/30 rounded-lg", class])) {
            span class="text-muted-foreground" data-icon="robot" { "robot" }
            div class="flex gap-1" {
                span class="w-2 h-2 rounded-full bg-muted-foreground animate-bounce" style="animation-delay: 0ms" {}
                span class="w-2 h-2 rounded-full bg-muted-foreground animate-bounce" style="animation-delay: 150ms" {}
                span class="w-2 h-2 rounded-full bg-muted-foreground animate-bounce" style="animation-delay: 300ms" {}
            }
        }
    }
}

#[must_use]
pub fn TimerDisplay(minutes: i32, seconds: i32, class: &str) -> Markup {
    html! { span class=(shared::build_classes(&["font-mono tabular-nums", class])) { (format!("{:02}:{:02}", minutes.max(0), seconds.max(0))) } }
}

#[must_use]
pub fn CountBadge(count: i32, label: &str, class: &str) -> Markup {
    html! {
        span class=(shared::build_classes(&["inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-xs font-semibold", class])) {
            (count)
            @if !label.is_empty() {
                span class="opacity-80" { (label) }
            }
        }
    }
}

#[must_use]
pub fn InlineNotification(notificationType: &str, title: &str, message: &str) -> Markup {
    html! {
        div class=(shared::build_classes(&["rounded-lg border p-4", getInlineNotificationClasses(notificationType)])) {
            div class="flex items-start gap-3" {
                span class="mt-0.5" { (inlineNotificationIcon(notificationType)) }
                div class="flex-1 space-y-1" {
                    @if !title.is_empty() {
                        p class="text-sm font-medium" { (title) }
                    }
                    p class="text-sm" { (message) }
                }
                button type="button" class="opacity-70 hover:opacity-100" aria-label="Dismiss" data-on:click="evt.currentTarget.closest('[class*=rounded-lg]').remove()" { "×" }
            }
        }
    }
}

#[must_use]
pub fn inlineNotificationIcon(notificationType: &str) -> Markup {
    let (name, glyph) = match notificationType {
        "success" => ("check-circle", "✓"),
        "error" => ("x-circle", "✕"),
        "warning" => ("alert-triangle", "⚠"),
        _ => ("info", "ℹ"),
    };
    html! { span data-icon=(name) { (glyph) } }
}

#[must_use]
pub fn CountBadgeComponent(props: &CountBadgeProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        @if props.should_show() {
            span class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
                (props.display_count())
                @if !props.label.is_empty() {
                    span class="sr-only" { (&props.label) }
                }
            }
        }
    }
}

#[must_use]
pub fn PendingCountBadge(count: i32, label: &str) -> Markup {
    CountBadgeComponent(&CountBadgeProps {
        count,
        label: label.to_string(),
        variant: shared::VARIANT_WARNING,
        pulsing: count > 0,
        ..Default::default()
    })
}

#[must_use]
pub fn CompletedCountBadge(count: i32, label: &str) -> Markup {
    CountBadgeComponent(&CountBadgeProps {
        count,
        label: label.to_string(),
        variant: shared::VARIANT_SUCCESS,
        show_zero: true,
        max_count: 999,
        ..Default::default()
    })
}

#[must_use]
pub fn UnreadCountBadge(count: i32) -> Markup {
    CountBadgeComponent(&CountBadgeProps {
        count,
        label: "Unread".to_string(),
        variant: shared::VARIANT_DESTRUCTIVE,
        pulsing: count > 0,
        ..Default::default()
    })
}

#[must_use]
pub fn PendingQuizzesBadge(count: i32) -> Markup {
    PendingCountBadge(count, "Pending quizzes")
}

#[must_use]
pub fn CompletedQuizzesBadge(count: i32) -> Markup {
    CompletedCountBadge(count, "Completed quizzes")
}

#[must_use]
pub fn UnreadMessagesBadge(count: i32) -> Markup {
    UnreadCountBadge(count)
}

#[must_use]
pub fn SessionCountBadge(count: i32) -> Markup {
    CountBadgeComponent(&CountBadgeProps {
        count,
        label: "Sessions".to_string(),
        variant: shared::VARIANT_PRIMARY,
        show_zero: true,
        max_count: 999,
        ..Default::default()
    })
}

#[must_use]
pub fn DocumentCountBadge(count: i32) -> Markup {
    CountBadgeComponent(&CountBadgeProps {
        count,
        label: "Documents".to_string(),
        variant: shared::VARIANT_INFO,
        show_zero: true,
        max_count: 999,
        ..Default::default()
    })
}

#[must_use]
pub fn FormLevelValidation(props: &FormLevelValidationProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) role=(props.role()) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-start gap-3" {
                span class=(shared::build_classes(&["mt-0.5", props.title_classes()])) { (props.icon()) }
                div class="flex-1 space-y-1" {
                    @if !props.title.is_empty() {
                        p class=(props.title_classes()) { (&props.title) }
                    }
                    @if !props.messages.is_empty() {
                        ul class=(shared::build_classes(&["text-sm space-y-1", props.list_classes()])) {
                            @for msg in &props.messages {
                                li class="flex items-center gap-2" {
                                    span class="text-xs" { "•" }
                                    span { (msg) }
                                }
                            }
                        }
                    }
                }
                @if props.dismissible {
                    button type="button" class="opacity-70 hover:opacity-100" data-on:click="evt.currentTarget.closest('[role]').remove()" { "×" }
                }
            }
        }
    }
}

#[must_use]
pub fn FormLevelError(title: &str, messages: &[&str]) -> Markup {
    FormLevelValidation(&FormLevelValidationProps {
        title: title.to_string(),
        messages: messages.iter().map(|value| (*value).to_string()).collect(),
        is_error: true,
        ..Default::default()
    })
}

#[must_use]
pub fn FormLevelSuccess(title: &str, messages: &[&str]) -> Markup {
    FormLevelValidation(&FormLevelValidationProps {
        title: title.to_string(),
        messages: messages.iter().map(|value| (*value).to_string()).collect(),
        is_error: false,
        ..Default::default()
    })
}

#[must_use]
pub fn SimpleFormError(message: &str) -> Markup {
    FormLevelError("Error", &[message])
}

#[must_use]
pub fn SimpleFormSuccess(message: &str) -> Markup {
    FormLevelSuccess("Success", &[message])
}

#[must_use]
pub fn FormValidationSummary(title: &str, fieldErrors: &[(&str, &str)]) -> Markup {
    html! {
        @if !fieldErrors.is_empty() {
            div class="rounded-lg border border-destructive/50 bg-destructive/10 p-4" role="alert" {
                div class="flex items-start gap-3" {
                    span class="mt-0.5 font-semibold text-destructive" { "✕" }
                    div class="flex-1 space-y-1" {
                        @if !title.is_empty() {
                            p class="font-semibold text-destructive" { (title) }
                        }
                        ul class="text-sm space-y-1 text-destructive/90" {
                            @for (field, msg) in fieldErrors {
                                li class="flex items-center gap-2" {
                                    span class="text-xs" { "•" }
                                    span class="font-medium" { (field) ":" }
                                    span { (msg) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn XPProgressFragment(currentXP: i32, requiredXP: i32, level: i32) -> Markup {
    html! {
        div class="space-y-2" {
            div class="flex items-center justify-between text-sm" {
                span class="text-muted-foreground" { "Level " (level) }
                span class="font-medium" { (currentXP) " / " (requiredXP) " XP" }
            }
            div class="h-2 bg-muted rounded-full overflow-hidden" {
                div class="h-full bg-gradient-to-r from-primary to-primary/70 rounded-full transition-all duration-500" style=(format!("width: {}%", clampedPercent(currentXP, requiredXP))) {}
            }
        }
    }
}

#[must_use]
pub fn StreakFragment(currentStreak: i32, bestStreak: i32) -> Markup {
    html! {
        div class="inline-flex items-center gap-2 px-3 py-1.5 rounded-full bg-accent/20" {
            @if currentStreak > 0 {
                (iconOrSymbol("🔥", "", shared::SIZE_SM))
            }
            span class="font-bold" { (currentStreak) }
            span class="text-xs text-muted-foreground" { "day streak" }
            @if bestStreak > currentStreak {
                span class="text-xs text-muted-foreground" { "best " (bestStreak) }
            }
        }
    }
}

#[must_use]
pub fn ScoreFragment(score: i32, maxScore: i32) -> Markup {
    html! {
        span class=(shared::build_classes(&["inline-flex items-center gap-1 px-2.5 py-1 rounded-full text-sm font-semibold", getScoreClasses(score, maxScore)])) {
            (percent(score, maxScore)) "%"
        }
    }
}

#[must_use]
pub fn OnlineStatusFragment(isOnline: bool, label: &str) -> Markup {
    html! {
        span class="inline-flex items-center gap-2 px-2 py-0.5 rounded-full text-xs font-medium" {
            span class=(shared::build_classes(&["h-2 w-2 rounded-full", getOnlineColor(isOnline)])) {}
            @if label.is_empty() {
                span { (getOnlineLabel(isOnline)) }
            } @else {
                span { (label) }
            }
        }
    }
}

#[must_use]
pub fn MasteryFragment(mastery: i32, topic: &str) -> Markup {
    html! {
        div class="space-y-1" {
            div class="flex items-center justify-between text-xs" {
                span class="text-muted-foreground truncate max-w-[150px]" { (topic) }
                span class="font-medium" { (mastery) "%" }
            }
            div class="h-1.5 bg-muted rounded-full overflow-hidden" {
                div class=(shared::build_classes(&["h-full rounded-full transition-all duration-300", getMasteryColor(mastery)])) style=(format!("width: {}%", mastery.clamp(0, 100))) {}
            }
        }
    }
}

#[must_use]
pub fn QuizResultFragment(score: i32, total: i32, passed: bool) -> Markup {
    html! {
        div class=(shared::build_classes(&["p-4 rounded-lg border", getQuizResultClasses(passed)])) {
            div class="flex items-center gap-3" {
                (iconOrSymbol(if passed { "🎉" } else { "📚" }, "", shared::SIZE_LG))
                div {
                    p class="font-semibold" { @if passed { "Quiz passed" } @else { "Quiz failed" } }
                    p class="text-sm text-muted-foreground" { "Score " (score) " / " (total) " (" (percent(score, total)) "%)" }
                }
            }
        }
    }
}

#[must_use]
pub fn ItemProgress(props: &ItemProgressProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-center justify-between gap-2" {
                div class="flex items-center gap-2 flex-1 min-w-0" {
                    span class=(props.status_color()) { (props.status_icon()) }
                    span class="text-sm font-medium truncate" { (&props.title) }
                }
                @if !props.badge.is_empty() {
                    (CountBadgeComponent(&CountBadgeProps { count: 0, label: props.badge.clone(), variant: props.badge_variant, show_zero: true, ..Default::default() }))
                }
            }
            @if !props.subtitle.is_empty() {
                p class="text-xs text-muted-foreground mt-1" { (&props.subtitle) }
            }
            @if props.is_processing() {
                div class="h-1.5 bg-muted rounded-full overflow-hidden mt-2" {
                    div class="h-full bg-primary rounded-full transition-all duration-300" style=(format!("width: {}%", props.percent.clamp(0, 100))) {}
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleItemProgress(title: &str, status: &str, percent: i32) -> Markup {
    ItemProgress(&ItemProgressProps {
        title: title.to_string(),
        status: status.to_string(),
        percent,
        ..Default::default()
    })
}

#[must_use]
pub fn ItemProgressWithBadge(title: &str, status: &str, badge: &str) -> Markup {
    ItemProgress(&ItemProgressProps {
        title: title.to_string(),
        status: status.to_string(),
        badge: badge.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn FileProcessingItem(filename: &str, status: &str, percent: i32) -> Markup {
    ItemProgress(&ItemProgressProps {
        title: filename.to_string(),
        status: status.to_string(),
        percent,
        icon: "📄".to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn GenerationProgress(props: &GenerationProgressProps) -> Markup {
    let attrs = props.build_attributes();
    let title = if !props.title.is_empty() {
        props.title.clone()
    } else if props.is_generating() {
        "Generating".to_string()
    } else if props.status == "complete" {
        "Complete".to_string()
    } else {
        "Error".to_string()
    };
    html! {
        div class=(shared::build_classes(&["space-y-2 rounded-lg border bg-card p-4", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-center gap-2" {
                @if props.is_generating() {
                    (SimpleSpinner(""))
                } @else {
                    span class=(props.text_classes()) { (props.status_icon()) }
                }
                span class=(shared::build_classes(&["text-sm font-medium", props.text_classes()])) { (title) }
            }
            @if !props.subtitle.is_empty() {
                div class=(shared::build_classes(&["text-xs", props.muted_text_classes()])) { (&props.subtitle) }
            }
            @if !props.message.is_empty() || props.items_count > 0 {
                div class="flex items-center justify-between text-sm" {
                    @if !props.message.is_empty() {
                        span class=(props.muted_text_classes()) { (&props.message) }
                    }
                    @if props.items_count > 0 {
                        span class=(shared::build_classes(&["font-medium", props.text_classes()])) { (props.format_items_count()) }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleGenerationProgress(title: &str, status: &str) -> Markup {
    GenerationProgress(&GenerationProgressProps {
        title: title.to_string(),
        status: status.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn GenerationInProgress(
    title: &str,
    message: &str,
    itemsCount: i32,
    itemsLabel: &str,
) -> Markup {
    GenerationProgress(&GenerationProgressProps {
        title: title.to_string(),
        status: "generating".to_string(),
        message: message.to_string(),
        items_count: itemsCount,
        items_label: itemsLabel.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn GenerationCompleteWithItems(
    title: &str,
    subtitle: &str,
    itemsCount: i32,
    itemsLabel: &str,
) -> Markup {
    GenerationProgress(&GenerationProgressProps {
        title: title.to_string(),
        subtitle: subtitle.to_string(),
        status: "complete".to_string(),
        items_count: itemsCount,
        items_label: itemsLabel.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn GenerationError(title: &str, message: &str) -> Markup {
    GenerationProgress(&GenerationProgressProps {
        title: title.to_string(),
        status: "error".to_string(),
        message: message.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn ToastContainer(position: &str) -> Markup {
    html! { div id="toast-container" class=(getToastContainerClasses(position)) {} }
}

#[must_use]
pub fn ToastSuccessPositioned(message: &str, position: &str) -> Markup {
    html! { div id="toast-container" class=(getToastContainerClasses(position)) { (ToastSuccess(message)) } }
}

#[must_use]
pub fn ToastErrorPositioned(message: &str, position: &str) -> Markup {
    html! { div id="toast-container" class=(getToastContainerClasses(position)) { (ToastError(message)) } }
}

#[must_use]
pub fn StreamingContent(props: &StreamingContentProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["rounded-lg border bg-card p-4", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-start gap-3" {
                @if !props.icon.is_empty() {
                    (iconOrSymbol(&props.icon, "text-lg", shared::SIZE_MD))
                }
                div class="flex-1 prose prose-sm dark:prose-invert max-w-none" {
                    (&props.content)
                    @if props.should_show_cursor() {
                        span class="inline-block w-2 h-4 bg-primary animate-pulse ml-1" {}
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleStreamingContent(content: &str, isComplete: bool) -> Markup {
    StreamingContent(&StreamingContentProps {
        content: content.to_string(),
        is_complete: isComplete,
        show_cursor: true,
        ..Default::default()
    })
}

#[must_use]
pub fn StreamingContentWithIcon(icon: &str, content: &str, isComplete: bool) -> Markup {
    StreamingContent(&StreamingContentProps {
        icon: icon.to_string(),
        content: content.to_string(),
        is_complete: isComplete,
        show_cursor: true,
        ..Default::default()
    })
}

#[must_use]
pub fn ProgressCounter(props: &ProgressCounterProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["space-y-1", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            span class="text-sm text-muted-foreground" { (props.format_progress()) }
            @if props.show_bar {
                div class="h-1.5 bg-muted rounded-full overflow-hidden mt-1" {
                    div class="h-full bg-primary rounded-full transition-all duration-300" style=(format!("width: {}%", props.progress_percent())) {}
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleProgressCounter(current: i32, total: i32, label: &str) -> Markup {
    ProgressCounter(&ProgressCounterProps {
        current,
        total,
        label: label.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn ProgressCounterWithBar(current: i32, total: i32, label: &str) -> Markup {
    ProgressCounter(&ProgressCounterProps {
        current,
        total,
        label: label.to_string(),
        show_bar: true,
        ..Default::default()
    })
}

#[must_use]
pub fn ChatBubble(props: &ChatBubbleProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.wrapper_classes()) {
            div class=(props.bubble_classes()) data-testid=[attr(&attrs, "data-testid")] {
                p class="text-sm whitespace-pre-wrap" { (&props.content) }
            }
        }
    }
}

#[must_use]
pub fn ChatBubbleOutgoing(content: &str) -> Markup {
    ChatBubble(&ChatBubbleProps {
        content: content.to_string(),
        is_outgoing: true,
        ..Default::default()
    })
}

#[must_use]
pub fn ChatBubbleIncoming(content: &str) -> Markup {
    ChatBubble(&ChatBubbleProps {
        content: content.to_string(),
        is_outgoing: false,
        ..Default::default()
    })
}

#[must_use]
pub fn StatCard(props: &StatCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-center gap-4" {
                @if !props.icon.is_empty() {
                    div class="text-3xl" { (iconOrSymbol(&props.icon, "", shared::SIZE_XL)) }
                }
                div {
                    @if !props.title.is_empty() {
                        p class="text-sm font-medium text-muted-foreground" { (&props.title) }
                    }
                    p class="text-3xl font-bold text-foreground" { (&props.value) }
                    @if !props.subtitle.is_empty() {
                        p class="text-xs text-muted-foreground" { (&props.subtitle) }
                    }
                    @if !props.trend_value.is_empty() {
                        span class=(shared::build_classes(&["text-xs font-medium", props.trend_color()])) { (props.trend_icon()) " " (&props.trend_value) }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleStatCard(title: &str, value: &str, icon: &str) -> Markup {
    StatCard(&StatCardProps {
        title: title.to_string(),
        value: value.to_string(),
        icon: icon.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn StatCardWithTrend(
    title: &str,
    value: &str,
    icon: &str,
    trend: &str,
    trendValue: &str,
) -> Markup {
    StatCard(&StatCardProps {
        title: title.to_string(),
        value: value.to_string(),
        icon: icon.to_string(),
        trend: trend.to_string(),
        trend_value: trendValue.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn NeomorphStatCard(title: &str, value: &str, icon: &str) -> Markup {
    StatCard(&StatCardProps {
        title: title.to_string(),
        value: value.to_string(),
        icon: icon.to_string(),
        class: "neomorph-flat".to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn Pagination(props: &PaginationProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        @if props.total_pages > 1 {
            nav class=(props.build_classes()) aria-label="Pagination" data-testid=[attr(&attrs, "data-testid")] {
                @if props.show_prev_next {
                    @if props.has_prev() {
                        a href=(props.get_page_url(props.current_page - 1)) class="px-3 py-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors duration-150" { "‹" }
                    } @else {
                        span class="px-3 py-1.5 text-sm text-muted-foreground/50 cursor-not-allowed" { "‹" }
                    }
                }
                @if props.show_numbers {
                    @for page in props.visible_pages() {
                        @if page == props.current_page {
                            span class="px-3 py-1.5 text-sm bg-foreground text-background rounded-lg font-medium" { (page) }
                        } @else {
                            a href=(props.get_page_url(page)) class="px-3 py-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors duration-150" { (page) }
                        }
                    }
                }
                @if props.show_prev_next {
                    @if props.has_next() {
                        a href=(props.get_page_url(props.current_page + 1)) class="px-3 py-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors duration-150" { "›" }
                    } @else {
                        span class="px-3 py-1.5 text-sm text-muted-foreground/50 cursor-not-allowed" { "›" }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimplePagination(currentPage: i32, totalPages: i32, baseURL: &str) -> Markup {
    Pagination(&PaginationProps {
        current_page: currentPage,
        total_pages: totalPages,
        base_url: baseURL.to_string(),
        show_prev_next: true,
        show_numbers: true,
        max_visible: 5,
        ..Default::default()
    })
}

#[must_use]
pub fn CompactPagination(currentPage: i32, totalPages: i32, baseURL: &str) -> Markup {
    Pagination(&PaginationProps {
        current_page: currentPage,
        total_pages: totalPages,
        base_url: baseURL.to_string(),
        show_prev_next: true,
        show_numbers: false,
        ..Default::default()
    })
}

#[must_use]
pub fn SimpleStat(props: &SimpleStatProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            span class=(props.value_classes()) { (&props.value) }
            @if !props.label.is_empty() {
                span class="text-sm text-muted-foreground" { (&props.label) }
            }
        }
    }
}

#[must_use]
pub fn SimpleStatValue(value: &str, label: &str) -> Markup {
    SimpleStat(&SimpleStatProps {
        value: value.to_string(),
        label: label.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn SimpleStatAligned(value: &str, label: &str, align: &str) -> Markup {
    SimpleStat(&SimpleStatProps {
        value: value.to_string(),
        label: label.to_string(),
        align: align.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn IconStatCard(props: &IconStatCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-center gap-4" {
                @if !props.icon.is_empty() {
                    div class="text-3xl" { (iconOrSymbol(&props.icon, "", shared::SIZE_XL)) }
                }
                div {
                    @if !props.label.is_empty() {
                        p class="text-sm font-medium text-muted-foreground" { (&props.label) }
                    }
                    p class="text-3xl font-bold text-foreground" { (&props.value) }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleIconStatCard(label: &str, value: &str, icon: &str) -> Markup {
    IconStatCard(&IconStatCardProps {
        label: label.to_string(),
        value: value.to_string(),
        icon: icon.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn NeomorphIconStatCard(label: &str, value: &str, icon: &str) -> Markup {
    IconStatCard(&IconStatCardProps {
        label: label.to_string(),
        value: value.to_string(),
        icon: icon.to_string(),
        style: "neomorph".to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn userListItemContent(props: &UserListItemProps) -> Markup {
    html! {
        div class="flex items-center gap-4" {
            div class="relative flex-shrink-0" {
                @if !props.avatar_src.is_empty() {
                    img src=(&props.avatar_src) alt=(&props.name) class="h-12 w-12 rounded-full object-cover";
                } @else {
                    div class="h-12 w-12 rounded-full bg-primary/10 flex items-center justify-center text-lg font-semibold text-primary" { (props.initials()) }
                }
                @if props.show_online {
                    (SimpleOnlineIndicator(props.is_online, "absolute bottom-0 right-0"))
                }
            }
            div class="flex-1 min-w-0" {
                p class="font-medium truncate" { (&props.name) }
                @if !props.subtitle.is_empty() {
                    p class="text-sm text-muted-foreground truncate" { (&props.subtitle) }
                }
                @if !props.stats.is_empty() {
                    div class="flex gap-2 text-xs text-muted-foreground mt-1" {
                        @for stat in &props.stats {
                            span { (stat) }
                        }
                    }
                }
            }
            @if !props.value.is_empty() {
                div class="text-right flex-shrink-0" {
                    p class="text-lg font-bold" { (&props.value) }
                    @if !props.value_label.is_empty() {
                        p class="text-xs text-muted-foreground" { (&props.value_label) }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn UserListItem(props: &UserListItemProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        @if props.href.is_empty() {
            div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] { (userListItemContent(props)) }
        } @else {
            a href=(&props.href) class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] { (userListItemContent(props)) }
        }
    }
}

#[must_use]
pub fn SimpleUserListItem(name: &str, subtitle: &str, href: &str) -> Markup {
    UserListItem(&UserListItemProps {
        name: name.to_string(),
        subtitle: subtitle.to_string(),
        href: href.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn UserListItemWithStats(
    name: &str,
    stats: &[&str],
    value: &str,
    valueLabel: &str,
    href: &str,
) -> Markup {
    UserListItem(&UserListItemProps {
        name: name.to_string(),
        stats: stats.iter().map(|value| (*value).to_string()).collect(),
        value: value.to_string(),
        value_label: valueLabel.to_string(),
        href: href.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn RankingCard(props: &RankingCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["rounded-lg border bg-card", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-center justify-between p-4" {
                div class="flex items-center gap-4" {
                    div class=(shared::build_classes(&["flex items-center justify-center w-10 h-10 rounded-full font-bold", getRankClasses(props.rank)])) { "#" (props.rank) }
                    div {
                        p class="font-medium" { (&props.name) }
                        @if !props.subtitle.is_empty() {
                            p class="text-sm text-muted-foreground" { (&props.subtitle) }
                        }
                    }
                }
                div class="text-right" {
                    p class="text-lg font-bold" { (props.value) }
                    @if !props.value_label.is_empty() { p class="text-xs text-muted-foreground" { (&props.value_label) } }
                    @if !props.secondary.is_empty() { p class="text-xs text-muted-foreground" { (&props.secondary) } }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleRankingCard(rank: i32, name: &str, value: i32, valueLabel: &str) -> Markup {
    RankingCard(&RankingCardProps {
        rank,
        name: name.to_string(),
        value,
        value_label: valueLabel.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn SectionCard(props: &SectionCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["rounded-lg border bg-card", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            @if !props.title.is_empty() {
                div class="flex items-center gap-2 px-6 py-4 border-b" {
                    @if !props.icon.is_empty() {
                        span class="text-lg" { (iconOrSymbol(&props.icon, "", shared::SIZE_MD)) }
                    }
                    h3 class="text-lg font-semibold" { (&props.title) }
                    @if !props.subtitle.is_empty() {
                        span class="text-sm text-muted-foreground" { (&props.subtitle) }
                    }
                }
            }
            div class="p-6" {}
        }
    }
}

#[must_use]
pub fn SimpleSectionCard(title: &str) -> Markup {
    SectionCard(&SectionCardProps {
        title: title.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn SectionCardWithIcon(title: &str, icon: &str) -> Markup {
    SectionCard(&SectionCardProps {
        title: title.to_string(),
        icon: icon.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn MasteryDisplay(props: &MasteryDisplayProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["space-y-1", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-center justify-between" {
                @if !props.label.is_empty() {
                    span class="text-sm text-muted-foreground" { (&props.label) }
                }
                span class=(shared::build_classes(&["font-bold", getMasteryColor(props.percent).replace("bg-", "text-").as_str()])) { (props.percent) "%" }
            }
            @if props.show_bar {
                div class="h-1.5 bg-muted rounded-full overflow-hidden" {
                    div class=(shared::build_classes(&["h-full rounded-full transition-all duration-300", getMasteryColor(props.percent)])) style=(format!("width: {}%", props.percent.clamp(0, 100))) {}
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleMasteryDisplay(percent: i32, label: &str) -> Markup {
    MasteryDisplay(&MasteryDisplayProps {
        percent,
        label: label.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn MasteryDisplayWithBar(percent: i32, label: &str) -> Markup {
    MasteryDisplay(&MasteryDisplayProps {
        percent,
        label: label.to_string(),
        show_bar: true,
        ..Default::default()
    })
}

#[must_use]
pub fn ActivityRow(props: &ActivityRowProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["flex items-center justify-between py-2", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="text-sm font-medium" { (&props.date) }
            div class="flex items-center gap-4" {
                @for stat in &props.stats {
                    div class="text-right" {
                        span class=(shared::build_classes(&["text-sm", metaVariantClasses(&stat.variant)])) { (&stat.value) }
                        @if !stat.label.is_empty() {
                            span class="text-xs text-muted-foreground ml-1" { (&stat.label) }
                        }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleActivityRow(date: &str, stats: &[&str]) -> Markup {
    ActivityRow(&ActivityRowProps {
        date: date.to_string(),
        stats: stats
            .iter()
            .map(|value| ActivityRowStat {
                value: (*value).to_string(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    })
}

#[must_use]
pub fn TranscriptMessage(props: &TranscriptMessageProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["rounded-lg border bg-card p-4", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="flex items-center justify-between mb-2" {
                span class=(shared::build_classes(&["text-sm font-medium", props.label_class()])) { (props.label()) }
                @if !props.timestamp.is_empty() { time datetime=[optional(&props.timestamp_iso)] { (&props.timestamp) } }
            }
            @if !props.html_content.is_empty() {
                div class="prose prose-sm dark:prose-invert max-w-none rich-content" data-rich-content="true" { (&props.html_content) }
            } @else {
                p class="text-sm whitespace-pre-wrap" { (&props.content) }
            }
        }
    }
}

#[must_use]
pub fn SimpleTranscriptMessage(role: &str, content: &str) -> Markup {
    TranscriptMessage(&TranscriptMessageProps {
        role: role.to_string(),
        content: content.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn TranscriptMessageWithLabel(
    role: &str,
    roleLabel: &str,
    content: &str,
    timestamp: &str,
) -> Markup {
    TranscriptMessage(&TranscriptMessageProps {
        role: role.to_string(),
        role_label: roleLabel.to_string(),
        content: content.to_string(),
        timestamp: timestamp.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn ListItemLinkCard(props: &ListItemLinkCardProps) -> Markup {
    let attrs = props.build_attributes();
    let status_class = if props.status_class.is_empty() {
        statusTypeClasses(&props.status)
    } else {
        props.status_class.as_str()
    };
    html! {
        a href=(&props.href) class=(shared::build_classes(&["block rounded-lg border bg-card transition-colors hover:bg-muted/50", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            div class="p-4" {
                div class="flex items-start justify-between gap-2 mb-3" {
                    h3 class="text-base font-semibold text-foreground truncate flex-1" { (&props.title) }
                    @if !props.status.is_empty() {
                        span class=(shared::build_classes(&["flex-shrink-0 px-2 py-0.5 text-xs font-medium rounded-full", status_class])) { (&props.status) }
                    }
                }
                @if !props.stats.is_empty() {
                    div class="flex items-center gap-4 text-sm text-muted-foreground mb-2" {
                        @for stat in &props.stats {
                            span class="flex items-center gap-1.5" {
                                @if !stat.icon.is_empty() { (iconOrSymbol(&stat.icon, "", shared::SIZE_SM)) }
                                (&stat.value)
                                @if !stat.label.is_empty() { " " (&stat.label) }
                            }
                        }
                    }
                }
                div class="flex items-center justify-between text-sm" {
                    time { (&props.timestamp) }
                    @if props.show_arrow { span class="text-muted-foreground" { "→" } }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleListItemLinkCard(href: &str, title: &str, timestamp: &str) -> Markup {
    ListItemLinkCard(&ListItemLinkCardProps {
        href: href.to_string(),
        title: title.to_string(),
        timestamp: timestamp.to_string(),
        show_arrow: true,
        ..Default::default()
    })
}

#[must_use]
pub fn ListItemLinkCardWithStatus(
    href: &str,
    title: &str,
    status: &str,
    statusClass: &str,
    timestamp: &str,
) -> Markup {
    ListItemLinkCard(&ListItemLinkCardProps {
        href: href.to_string(),
        title: title.to_string(),
        status: status.to_string(),
        status_class: statusClass.to_string(),
        timestamp: timestamp.to_string(),
        show_arrow: true,
        ..Default::default()
    })
}

#[must_use]
pub fn StatIconRow(props: &StatIconRowProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(shared::build_classes(&["flex items-center gap-4 text-sm text-muted-foreground", props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            @for stat in &props.stats {
                span class="flex items-center gap-1.5" {
                    @if !stat.icon.is_empty() { (iconOrSymbol(&stat.icon, "", shared::SIZE_SM)) }
                    span { (&stat.value) }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleStatIconRow(stats: &[&str]) -> Markup {
    StatIconRow(&StatIconRowProps {
        stats: stats
            .iter()
            .map(|value| IconStat {
                value: (*value).to_string(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    })
}

#[must_use]
pub fn StatusBadge(props: &StatusBadgeProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        span class=(shared::build_classes(&["px-2 py-0.5 text-xs font-medium rounded-full", statusTypeClasses(&props.status_type), props.class.as_str()])) data-testid=[attr(&attrs, "data-testid")] {
            (&props.status)
        }
    }
}

#[must_use]
pub fn SimpleStatusBadge(status: &str, statusType: &str) -> Markup {
    StatusBadge(&StatusBadgeProps {
        status: status.to_string(),
        status_type: statusType.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn ActiveStatusBadge(status: &str) -> Markup {
    SimpleStatusBadge(status, "active")
}

#[must_use]
pub fn CompletedStatusBadge(status: &str) -> Markup {
    SimpleStatusBadge(status, "completed")
}

#[must_use]
pub fn PendingStatusBadge(status: &str) -> Markup {
    SimpleStatusBadge(status, "pending")
}

#[must_use]
pub fn InactiveStatusBadge(status: &str) -> Markup {
    SimpleStatusBadge(status, "inactive")
}

#[must_use]
pub fn ItemInfoCard(props: &ItemInfoCardProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div class=(props.build_classes()) data-testid=[attr(&attrs, "data-testid")] {
            div class="p-4 space-y-2" {
                div class="flex items-center justify-between gap-2" {
                    div class="flex items-center gap-2" {
                        @if !props.icon.is_empty() { (iconOrSymbol(&props.icon, "text-xl", shared::SIZE_XL)) }
                        p class="text-sm font-semibold" { (&props.title) }
                    }
                    @if !props.badge.is_empty() {
                        span class=(props.badge_classes()) { (&props.badge) }
                    }
                }
                @if !props.description.is_empty() {
                    p class="text-sm text-muted-foreground" { (&props.description) }
                }
                @if !props.metadata.is_empty() {
                    div class="flex items-center justify-between text-sm" {
                        @for meta in &props.metadata {
                            span class=(metaVariantClasses(&meta.variant)) {
                                @if !meta.icon.is_empty() { (iconOrSymbol(&meta.icon, "", shared::SIZE_MD)) " " }
                                @if !meta.label.is_empty() { span class="text-muted-foreground" { (&meta.label) ": " } }
                                (&meta.value)
                            }
                        }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn SimpleItemInfoCard(icon: &str, title: &str, description: &str) -> Markup {
    ItemInfoCard(&ItemInfoCardProps {
        icon: icon.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        ..Default::default()
    })
}

#[must_use]
pub fn ItemInfoCardWithMeta(
    icon: &str,
    title: &str,
    description: &str,
    metadata: &[&str],
) -> Markup {
    ItemInfoCard(&ItemInfoCardProps {
        icon: icon.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        metadata: metadata
            .iter()
            .map(|value| ItemInfoMeta {
                value: (*value).to_string(),
                variant: "muted".to_string(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    })
}

#[must_use]
pub fn TrophyItemCard(
    icon: &str,
    title: &str,
    description: &str,
    xpValue: i32,
    earnedCount: i32,
) -> Markup {
    ItemInfoCard(&ItemInfoCardProps {
        icon: icon.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        metadata: vec![
            ItemInfoMeta {
                value: format!("{xpValue} XP"),
                variant: "primary".to_string(),
                ..Default::default()
            },
            ItemInfoMeta {
                value: format!("earned {earnedCount}"),
                variant: "muted".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    })
}

#[must_use]
pub fn LockedTrophyItemCard(
    icon: &str,
    title: &str,
    description: &str,
    requirement: &str,
) -> Markup {
    ItemInfoCard(&ItemInfoCardProps {
        icon: icon.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        badge: "Locked".to_string(),
        badge_class: "bg-muted text-muted-foreground".to_string(),
        is_disabled: true,
        metadata: vec![ItemInfoMeta {
            label: "Requirement".to_string(),
            value: requirement.to_string(),
            variant: "muted".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    })
}

#[must_use]
pub fn RewardItemCard(
    icon: &str,
    title: &str,
    description: &str,
    xpValue: i32,
    createdAt: &str,
) -> Markup {
    ItemInfoCard(&ItemInfoCardProps {
        icon: icon.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        metadata: vec![
            ItemInfoMeta {
                value: format!("{xpValue} XP"),
                variant: "primary".to_string(),
                ..Default::default()
            },
            ItemInfoMeta {
                value: createdAt.to_string(),
                variant: "muted".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_contains(html: &str, expected: &str) {
        assert!(html.contains(expected), "missing `{expected}` in `{html}`");
    }

    #[test]
    fn helpers_cover_branches() {
        assert_eq!(optional(""), None);
        assert_eq!(optional("x"), Some("x"));
        assert_eq!(boolStr(true), "true");
        assert_eq!(percent(3, 0), 0);
        assert_eq!(clampedPercent(150, 100), 100);
        assert_eq!(getOnlineLabel(false), "Offline");
        assert_eq!(onlineLabelKey(true), "component.indicators.online");
        assert_eq!(getNotificationLabel(2), "2 notifications");
        assert_eq!(resolveNotificationLabel(0), "No notifications");
        assert_eq!(getBellClass(true), "animate-wiggle");
        assert_eq!(getMenuLineTop(true), "top-1/2 -translate-y-1/2 rotate-45");
        assert_eq!(getMenuLineMiddle(false), "opacity-100");
        assert_eq!(
            getMenuLineBottom(true),
            "bottom-1/2 translate-y-1/2 -rotate-45"
        );
        assert_eq!(formatTime(3661, false), "01:01:01");
        assert_eq!(formatTimeWithHours(1, 2, 3), "01:02:03");
        assert_eq!(formatTimeMinSec(4, 5), "04:05");
        assert_eq!(padZero(12), "12");
        assert_eq!(padZero(-1), "00");
        assert_eq!(timerAriaLabel(0), "Timer complete");
        assert_eq!(getTimerTextClass(5, 100), "text-destructive animate-pulse");
        assert_eq!(getGenerationMessageClass("error"), "text-destructive");
        assert_eq!(getScoreClasses(80, 100), "bg-success-muted text-success");
        assert_eq!(getPercent(25, 50), 50);
        assert_eq!(getPercent(1, 0), 0);
        assert_eq!(getStatusDotColor(true), "bg-success");
        assert_eq!(getStatusDotColor(false), "bg-muted-foreground");
        assert_eq!(getMasteryColor(45), "bg-warning");
        assert_eq!(
            getQuizResultClasses(false),
            "border-warning bg-warning-muted"
        );
        assert_eq!(
            getInlineNotificationClasses("warning"),
            "border-warning bg-warning-muted text-warning"
        );
        assert_eq!(
            getToastContainerClasses("bottom-left"),
            "fixed z-50 animate-in slide-in-from-top-2 bottom-4 left-4"
        );
        assert_eq!(getRankClasses(2), "bg-muted text-muted-foreground");
        assert_eq!(statusTypeClasses("active"), "bg-success-muted text-success");
        assert_eq!(metaVariantClasses("primary"), "text-primary font-medium");
        assert_eq!(
            resolveMobileMenuAriaLabel(&MobileMenuButtonProps::default()),
            "Toggle menu"
        );
        assert_eq!(
            resolveMobileMenuAriaLabel(
                &MobileMenuButtonProps::default().with_aria_label("Open navigation")
            ),
            "Open navigation"
        );
        assert_eq!(resolveSpinnerLabel(""), "Loading...");
        assert_eq!(resolveSpinnerLabel("Working"), "Working");
        assert_eq!(resolveSVGSpinnerLabel(""), "Loading...");
        assert_eq!(resolveSVGSpinnerLabel("Drawing"), "Drawing");
        assert_eq!(getValidationRole(true), "alert");
        assert_eq!(getValidationRole(false), "status");
    }

    #[test]
    fn base_indicators_render_states() {
        let online = OnlineIndicator(
            &OnlineIndicatorProps::default()
                .with_online(true)
                .with_size(shared::SIZE_LG)
                .with_attribute("data-testid", "online"),
        )
        .into_string();
        assert_contains(&online, r#"aria-label="Online""#);
        assert_contains(&online, "animate-ping");
        assert_contains(&online, "h-4 w-4");
        assert_contains(&online, r#"data-testid="online""#);

        let bell = NotificationBell(
            &NotificationBellProps::default()
                .with_count(120)
                .with_max_count(99)
                .with_animating(true),
        )
        .into_string();
        assert_contains(&bell, r#"aria-label="120 notifications""#);
        assert_contains(&bell, "animate-wiggle");
        assert_contains(&bell, "99+");

        let menu = SimpleMobileMenuButton(true, "custom").into_string();
        assert_contains(&menu, r#"aria-expanded="true""#);
        assert_contains(&menu, "rotate-45");
        assert_contains(&menu, "custom");

        let progress = AnimatedProgress(
            &AnimatedProgressProps::default()
                .with_value(25)
                .with_max(50)
                .with_show_label(true)
                .with_variant(shared::VARIANT_SUCCESS),
        )
        .into_string();
        assert_contains(&progress, r#"role="progressbar""#);
        assert_contains(&progress, r#"aria-valuenow="25""#);
        assert_contains(&progress, "width: 50%");
        assert_contains(&progress, "bg-success");
    }

    #[test]
    fn spinner_timer_and_generation_render_variations() {
        let spinner = Spinner(
            &SpinnerProps::default()
                .with_size(shared::SIZE_XL)
                .with_label("Working")
                .with_class("spin-extra"),
        )
        .into_string();
        assert_contains(&spinner, r#"role="status""#);
        assert_contains(&spinner, r#"aria-label="Working""#);
        assert_contains(&spinner, "h-12 w-12");
        assert_contains(&spinner, "spin-extra");

        let svg = SimpleSVGSpinner(shared::SIZE_SM, "svg-extra").into_string();
        assert_contains(&svg, "<svg");
        assert_contains(&svg, "w-3.5 h-3.5");
        assert_contains(&svg, "<title>Loading...</title>");

        let timer = QuizTimer(100, 8, true).into_string();
        assert_contains(&timer, r#"role="timer""#);
        assert_contains(&timer, "00:08");
        assert_contains(&timer, "Running");
        assert_contains(&timer, "text-destructive");

        let complete = SimpleGenerationStateIndicator("complete", "Done").into_string();
        assert_contains(&complete, r#"data-icon="check""#);
        assert_contains(&complete, "Done");
        let error = SimpleGenerationStateIndicator("error", "Nope").into_string();
        assert_contains(&error, r#"data-icon="x-circle""#);
        assert_contains(&error, "text-destructive");
    }

    #[test]
    fn processing_stats_validation_toasts_and_activity_render() {
        let processing = SimpleProcessingProgress("Parsing", 35).into_string();
        assert_contains(&processing, "Parsing");
        assert_contains(&processing, "width: 35%");
        assert_contains(
            &ProcessingSpinnerMessage("Waiting", "box").into_string(),
            "Waiting",
        );
        assert_contains(
            &ProcessingComplete("Finished", "done").into_string(),
            "Finished",
        );
        assert_contains(&ProcessingError("Failed", "err").into_string(), "Failed");
        assert_contains(
            &iconOrSymbol("⭐", "icon", shared::SIZE_SM).into_string(),
            r#"data-icon="⭐""#,
        );

        let stat = StatFragmentWithIcon("42", "Answers", "check").into_string();
        assert_contains(&stat, "42");
        assert_contains(&stat, "Answers");
        assert_contains(&stat, r#"data-icon="check""#);

        let error = ValidationError("email", "Required").into_string();
        assert_contains(&error, r#"id="email-error""#);
        assert_contains(&error, "Required");
        assert_contains(&ValidationSuccess("email").into_string(), "text-success");

        let toast = ToastWarning("Heads up").into_string();
        assert_contains(&toast, "Heads up");
        assert_contains(
            &toast,
            r#"data-on:click="evt.currentTarget.parentElement.remove()""#,
        );
        assert_contains(&ToastSuccess("Saved").into_string(), "text-success");
        assert_contains(&ToastError("Bad").into_string(), "text-destructive");
        assert_contains(&ToastInfo("Info").into_string(), "text-info");

        let activity = ActivityItem(&ActivityItemProps {
            username: "Ada".to_string(),
            action: "updated".to_string(),
            target: "Quiz".to_string(),
            timestamp: "now".to_string(),
            icon: "edit".to_string(),
            ..Default::default()
        })
        .into_string();
        assert_contains(&activity, "Ada");
        assert_contains(&activity, "Quiz");
        assert_contains(
            &TypingIndicator("typing").into_string(),
            "animation-delay: 300ms",
        );
        assert_contains(&TimerDisplay(2, 9, "clock").into_string(), "02:09");
        assert_contains(&CountBadge(5, "new", "badge").into_string(), "new");
        assert_contains(
            &InlineNotification("error", "Title", "Body").into_string(),
            r#"data-on:click="evt.currentTarget.closest('[class*=rounded-lg]').remove()""#,
        );
    }

    #[test]
    fn count_badges_and_form_validation_render_variations() {
        assert!(
            CountBadgeComponent(&CountBadgeProps::default())
                .into_string()
                .is_empty()
        );
        let maxed = CountBadgeComponent(&CountBadgeProps {
            count: 150,
            max_count: 99,
            label: "Inbox".to_string(),
            show_zero: true,
            ..Default::default()
        })
        .into_string();
        assert_contains(&maxed, "99+");
        assert_contains(&maxed, "Inbox");
        assert_contains(
            &PendingCountBadge(2, "Pending").into_string(),
            "animate-pulse",
        );
        let pending = NewPendingCountBadge(2, "Pending");
        assert_eq!(pending.count_badge_props.count, 2);
        assert_eq!(pending.count_badge_props.label, "Pending");
        assert_eq!(pending.count_badge_props.variant, shared::VARIANT_WARNING);
        assert!(pending.count_badge_props.pulsing);
        assert_contains(&CompletedCountBadge(0, "Done").into_string(), "0");
        let completed = NewCompletedCountBadge(0, "Done");
        assert_eq!(completed.count_badge_props.count, 0);
        assert_eq!(completed.count_badge_props.label, "Done");
        assert_eq!(completed.count_badge_props.variant, shared::VARIANT_SUCCESS);
        assert!(completed.count_badge_props.show_zero);
        assert_eq!(completed.count_badge_props.max_count, 999);
        assert_contains(&UnreadCountBadge(3).into_string(), "Unread");
        let unread = NewUnreadCountBadge(3);
        assert_eq!(unread.count_badge_props.count, 3);
        assert_eq!(
            unread.count_badge_props.variant,
            shared::VARIANT_DESTRUCTIVE
        );
        assert!(unread.count_badge_props.pulsing);
        assert_contains(&PendingQuizzesBadge(1).into_string(), "Pending quizzes");
        assert_contains(&CompletedQuizzesBadge(0).into_string(), "Completed quizzes");
        assert_contains(&UnreadMessagesBadge(4).into_string(), "Unread");
        assert_contains(&SessionCountBadge(0).into_string(), "Sessions");
        assert_contains(&DocumentCountBadge(0).into_string(), "Documents");

        let form_error = FormLevelError("Fix fields", &["Email", "Password"]).into_string();
        assert_contains(&form_error, r#"role="alert""#);
        assert_contains(&form_error, "Fix fields");
        assert_contains(
            &form_error,
            r#"data-on:click="evt.currentTarget.closest('[role]').remove()""#,
        );
        let error_props = NewFormLevelError("Fix fields", vec!["Email".to_string()]);
        assert_eq!(error_props.form_level_validation_props.title, "Fix fields");
        assert_eq!(
            error_props.form_level_validation_props.messages,
            vec!["Email".to_string()]
        );
        assert!(error_props.form_level_validation_props.is_error);
        let form_success = FormLevelSuccess("Saved", &["Profile"]).into_string();
        assert_contains(&form_success, r#"role="status""#);
        assert_contains(&form_success, "Profile");
        let success_props = NewFormLevelSuccess("Saved", vec!["Profile".to_string()]);
        assert_eq!(success_props.form_level_validation_props.title, "Saved");
        assert_eq!(
            success_props.form_level_validation_props.messages,
            vec!["Profile".to_string()]
        );
        assert!(!success_props.form_level_validation_props.is_error);
        assert_contains(&SimpleFormError("No").into_string(), "Error");
        assert_contains(&SimpleFormSuccess("Yes").into_string(), "Success");
        let summary = FormValidationSummary("Errors", &[("email", "bad")]).into_string();
        assert_contains(&summary, "email");
        assert_contains(&summary, "bad");
        assert!(FormValidationSummary("None", &[]).into_string().is_empty());
    }

    #[test]
    fn sse_fragments_render_progress_and_results() {
        assert_contains(&XPProgressFragment(50, 100, 3).into_string(), "Level 3");
        assert_contains(&StreakFragment(5, 10).into_string(), "best 10");
        assert_contains(&ScoreFragment(7, 10).into_string(), "70%");
        assert_contains(&OnlineStatusFragment(false, "").into_string(), "Offline");
        assert_contains(&MasteryFragment(85, "Rust").into_string(), "Rust");
        assert_contains(
            &QuizResultFragment(8, 10, true).into_string(),
            "Quiz passed",
        );

        let item = SimpleItemProgress("Document", "processing", 60).into_string();
        assert_contains(&item, "Document");
        assert_contains(&item, "width: 60%");
        assert_contains(
            &ItemProgressWithBadge("Task", "complete", "New").into_string(),
            "New",
        );
        assert_contains(
            &FileProcessingItem("notes.pdf", "uploading", 20).into_string(),
            "notes.pdf",
        );

        let in_progress = GenerationInProgress("Generate", "Working", 6, "questions").into_string();
        assert_contains(&in_progress, "Generate");
        assert_contains(&in_progress, "6 questions");
        assert_contains(
            &GenerationCompleteWithItems("Done", "Ready", 2, "items").into_string(),
            "Ready",
        );
        assert_contains(
            &GenerationError("Failed", "Try again").into_string(),
            "Try again",
        );
    }

    #[test]
    fn positioned_streaming_counters_chat_and_stats_render() {
        assert_contains(
            &ToastContainer("top-center").into_string(),
            "top-4 left-1/2",
        );
        assert_contains(
            &ToastSuccessPositioned("Saved", "bottom-right").into_string(),
            "Saved",
        );
        assert_contains(
            &ToastErrorPositioned("Bad", "bottom-left").into_string(),
            "Bad",
        );

        let streaming = SimpleStreamingContent("<b>escaped</b>", false).into_string();
        assert_contains(&streaming, "&lt;b&gt;escaped&lt;/b&gt;");
        assert_contains(&streaming, "animate-pulse");
        assert!(
            !SimpleStreamingContent("done", true)
                .into_string()
                .contains("inline-block w-2")
        );
        assert_contains(
            &StreamingContentWithIcon("sparkles", "Hi", false).into_string(),
            r#"data-icon="sparkles""#,
        );

        assert_contains(
            &SimpleProgressCounter(2, 5, "files").into_string(),
            "2 / 5 files",
        );
        assert_contains(
            &ProgressCounterWithBar(2, 5, "files").into_string(),
            "width: 40%",
        );
        assert_contains(&ChatBubbleOutgoing("Hello").into_string(), "justify-end");
        assert_contains(&ChatBubbleIncoming("Hi").into_string(), "justify-start");

        assert_contains(
            &SimpleStatCard("Users", "12", "user").into_string(),
            "Users",
        );
        assert_contains(
            &StatCardWithTrend("Sales", "9", "chart", "up", "+5%").into_string(),
            "↑ +5%",
        );
        assert_contains(
            &NeomorphStatCard("XP", "100", "star").into_string(),
            "neomorph-flat",
        );
        assert_contains(
            &SimpleIconStatCard("Files", "7", "file").into_string(),
            "Files",
        );
        assert_contains(
            &NeomorphIconStatCard("Files", "7", "file").into_string(),
            "neomorph-flat",
        );
        assert_contains(&SimpleStatValue("42", "Total").into_string(), "Total");
        assert_contains(
            &SimpleStatAligned("42", "Total", "right").into_string(),
            "items-end",
        );
    }

    #[test]
    fn pagination_user_ranking_section_and_mastery_render() {
        let pages = SimplePagination(2, 4, "/items").into_string();
        assert_contains(&pages, r#"<nav"#);
        assert_contains(&pages, r#"href="/items?page=1""#);
        assert_contains(&pages, r#"href="/items?page=3""#);
        assert_contains(
            &CompactPagination(2, 4, "/items?filter=all").into_string(),
            r#"href="/items?filter=all&amp;page=1""#,
        );

        assert_contains(
            &SimpleUserListItem("Ada Lovelace", "Math", "/ada").into_string(),
            "AL",
        );
        assert_contains(
            &UserListItemWithStats("Grace", &["10 XP", "2 quizzes"], "99", "score", "/g")
                .into_string(),
            "2 quizzes",
        );
        assert_contains(&SimpleRankingCard(1, "Ada", 100, "XP").into_string(), "#1");
        assert_contains(&SimpleSectionCard("Overview").into_string(), "Overview");
        assert_contains(
            &SectionCardWithIcon("Overview", "book").into_string(),
            r#"data-icon="book""#,
        );
        assert_contains(&SimpleMasteryDisplay(70, "Rust").into_string(), "70%");
        assert_contains(
            &MasteryDisplayWithBar(70, "Rust").into_string(),
            "width: 70%",
        );
    }

    #[test]
    fn activity_transcript_links_status_and_item_cards_render() {
        assert_contains(
            &SimpleActivityRow("Today", &["10 XP", "2 files"]).into_string(),
            "2 files",
        );
        assert_contains(
            &SimpleTranscriptMessage("assistant", "Hello").into_string(),
            "Assistant",
        );
        assert_contains(
            &TranscriptMessageWithLabel("bot", "Tutor", "Hi", "now").into_string(),
            "Tutor",
        );
        let html_content = TranscriptMessage(&TranscriptMessageProps {
            html_content: "<b>safe escaped</b>".to_string(),
            ..Default::default()
        })
        .into_string();
        assert_contains(&html_content, r#"data-rich-content="true""#);
        assert_contains(&html_content, "&lt;b&gt;safe escaped&lt;/b&gt;");

        assert_contains(
            &SimpleListItemLinkCard("/lesson", "Lesson", "now").into_string(),
            r#"href="/lesson""#,
        );
        assert_contains(
            &ListItemLinkCardWithStatus("/lesson", "Lesson", "active", "bg-success", "now")
                .into_string(),
            "bg-success",
        );
        assert_contains(&SimpleStatIconRow(&["12", "34"]).into_string(), "34");
        assert_contains(
            &SimpleStatusBadge("Active", "active").into_string(),
            "text-success",
        );
        assert_contains(&ActiveStatusBadge("Active").into_string(), "text-success");
        assert_contains(&CompletedStatusBadge("Done").into_string(), "text-success");
        assert_contains(&PendingStatusBadge("Wait").into_string(), "text-warning");
        assert_contains(
            &InactiveStatusBadge("Off").into_string(),
            "text-muted-foreground",
        );

        assert_contains(
            &SimpleItemInfoCard("cup", "Trophy", "Win").into_string(),
            "Trophy",
        );
        assert_contains(
            &ItemInfoCardWithMeta("cup", "Trophy", "Win", &["100 XP", "today"]).into_string(),
            "today",
        );
        assert_contains(
            &TrophyItemCard("cup", "Trophy", "Win", 100, 3).into_string(),
            "earned 3",
        );
        assert_contains(
            &LockedTrophyItemCard("lock", "Locked", "Later", "Level 2").into_string(),
            "Requirement",
        );
        assert_contains(
            &RewardItemCard("gift", "Reward", "Claim", 50, "today").into_string(),
            "50 XP",
        );
    }

    #[test]
    fn remaining_helper_branches_props_and_shortcuts_validate_html() {
        assert_eq!(
            component("Probe"),
            ComponentInfo {
                family: "unstyled",
                module: "indicators",
                name: "Probe",
            }
        );
        assert_eq!(boolStr(false), "false");
        assert_eq!(percent(25, 100), 25);
        assert_eq!(clampedPercent(-5, 100), 0);
        assert_eq!(getOnlineLabel(true), "Online");
        assert_eq!(onlineLabelKey(false), "component.indicators.offline");
        assert_eq!(getOnlineColor(true), "bg-success");
        assert_eq!(getOnlineColor(false), "bg-muted-foreground");
        assert_eq!(getNotificationLabel(0), "No notifications");
        assert_eq!(getNotificationLabel(1), "1 notification");
        assert_eq!(getBellClass(false), "");
        assert_eq!(getMenuLineTop(false), "top-0");
        assert_eq!(getMenuLineMiddle(true), "opacity-0");
        assert_eq!(getMenuLineBottom(false), "bottom-0");
        assert_eq!(formatTime(-1, false), "00:00");
        assert_eq!(formatTime(61, false), "01:01");
        assert_eq!(timerAriaLabel(61), "1 minutes and 1 seconds remaining");
        assert_eq!(timerAriaLabel(9), "9 seconds remaining");
        assert_eq!(getTimerTextClass(10, 0), "");
        assert_eq!(getTimerTextClass(20, 100), "text-warning");
        assert_eq!(getTimerTextClass(80, 100), "");
        assert_eq!(
            getGenerationMessageClass("generating"),
            "text-muted-foreground"
        );
        assert_eq!(getGenerationMessageClass("complete"), "text-success");
        assert_eq!(getGenerationMessageClass("idle"), "text-foreground");
        assert_eq!(getScoreClasses(60, 100), "bg-warning-muted text-warning");
        assert_eq!(
            getScoreClasses(20, 100),
            "bg-destructive-muted text-destructive"
        );
        assert_eq!(getMasteryColor(90), "bg-success");
        assert_eq!(getMasteryColor(65), "bg-info");
        assert_eq!(getMasteryColor(10), "bg-muted-foreground");
        assert_eq!(
            getQuizResultClasses(true),
            "border-success bg-success-muted"
        );
        assert_eq!(
            getInlineNotificationClasses("success"),
            "border-success bg-success-muted text-success"
        );
        assert_eq!(
            getInlineNotificationClasses("error"),
            "border-destructive bg-destructive-muted text-destructive"
        );
        assert_eq!(
            getInlineNotificationClasses("info"),
            "border-info bg-info-muted text-info"
        );
        assert!(getToastContainerClasses("top-left").contains("top-4 left-4"));
        assert!(getToastContainerClasses("top-center").contains("top-4 left-1/2"));
        assert!(getToastContainerClasses("bottom-right").contains("bottom-4 right-4"));
        assert!(getToastContainerClasses("bottom-center").contains("bottom-4 left-1/2"));
        assert!(getToastContainerClasses("unknown").contains("top-4 right-4"));
        assert_eq!(getRankClasses(1), "bg-warning-muted text-warning");
        assert_eq!(getRankClasses(3), "bg-warning-muted text-warning");
        assert_eq!(
            statusTypeClasses("pending"),
            "bg-warning-muted text-warning"
        );
        assert_eq!(
            statusTypeClasses("error"),
            "bg-destructive-muted text-destructive"
        );
        assert_eq!(
            statusTypeClasses("inactive"),
            "bg-muted text-muted-foreground"
        );
        assert_eq!(
            statusTypeClasses("unknown"),
            "bg-muted text-muted-foreground"
        );
        assert_eq!(metaVariantClasses("success"), "text-success font-medium");
        assert_eq!(metaVariantClasses("warning"), "text-warning font-medium");
        assert_eq!(
            metaVariantClasses("destructive"),
            "text-destructive font-medium"
        );
        assert_eq!(metaVariantClasses("plain"), "text-muted-foreground");

        let online_default = OnlineIndicator(&OnlineIndicatorProps::default()).into_string();
        assert_contains(&online_default, "h-3 w-3");
        let online_small = OnlineIndicator(
            &OnlineIndicatorProps::default()
                .with_online(false)
                .with_size(shared::SIZE_SM)
                .with_class("online-extra")
                .with_attribute("data-testid", "offline"),
        )
        .into_string();
        assert_contains(&online_small, "h-2 w-2");
        assert_contains(&online_small, r#"aria-label="Offline""#);
        assert_contains(&online_small, "online-extra");
        assert!(!online_small.contains("animate-ping"));

        let bell_zero = NotificationBell(
            &NotificationBellProps::default()
                .with_count(0)
                .with_max_count(10)
                .with_animating(false)
                .with_class("bell-extra")
                .with_attribute("data-testid", "bell"),
        )
        .into_string();
        assert_contains(&bell_zero, r#"aria-label="No notifications""#);
        assert_contains(&bell_zero, "bell-extra");
        assert!(!bell_zero.contains("-top-1"));
        assert_eq!(
            NotificationBellProps::default()
                .with_count(5)
                .with_max_count(10)
                .display_count(),
            "5"
        );

        let menu = MobileMenuButton(
            &MobileMenuButtonProps::default()
                .with_open(false)
                .with_aria_label("Menu")
                .with_class("menu-extra")
                .with_attribute("data-testid", "menu"),
        )
        .into_string();
        assert_contains(&menu, r#"aria-label="Menu""#);
        assert_contains(&menu, r#"aria-expanded="false""#);
        assert_contains(&menu, "menu-extra");

        let mut progress_props = AnimatedProgressProps::default()
            .with_value(20)
            .with_max(80)
            .with_show_label(false)
            .with_variant(shared::VARIANT_WARNING)
            .with_size(shared::SIZE_SM)
            .with_class("progress-extra")
            .with_attribute("data-testid", "progress");
        progress_props.animate_on_load = false;
        assert_eq!(progress_props.progress_percent(), 25);
        assert!(!progress_props.bar_classes().contains("transition-all"));
        let progress_html = AnimatedProgress(&progress_props).into_string();
        assert_contains(&progress_html, "bg-warning");
        assert_contains(&progress_html, "h-1");
        assert_contains(&progress_html, r#"data-testid="progress""#);
        assert!(!progress_html.contains(">Progress</span>"));

        for (size, expected) in [
            (shared::SIZE_XS, "h-3.5 w-3.5"),
            (shared::SIZE_SM, "h-4 w-4"),
            (shared::SIZE_MD, "h-6 w-6"),
            (shared::SIZE_LG, "h-8 w-8"),
        ] {
            let spinner = SpinnerProps::default()
                .with_size(size)
                .with_variant(shared::VARIANT_WARNING)
                .with_label("Spin")
                .with_class("spinner-extra")
                .with_attribute("data-testid", size);
            assert!(spinner.build_classes().contains(expected));
            assert!(spinner.build_classes().contains("text-warning"));
            let html = Spinner(&spinner).into_string();
            assert_contains(&html, r#"aria-label="Spin""#);
            assert_contains(&html, "spinner-extra");
        }

        for (size, expected) in [
            (shared::SIZE_XS, "w-3 h-3"),
            (shared::SIZE_LG, "w-7 h-7"),
            (shared::SIZE_XL, "w-10 h-10"),
            ("unknown", "w-5 h-5"),
        ] {
            let svg = SVGSpinnerProps::default()
                .with_size(size)
                .with_label("SVG")
                .with_class("svg-extra")
                .with_attribute("data-testid", size);
            assert_eq!(svg.resolved_label(), "SVG");
            assert_eq!(svg.size_classes(), expected);
            let html = SVGSpinner(&svg).into_string();
            assert_contains(&html, expected);
            assert_contains(&html, r#"<title>SVG</title>"#);
        }
        assert_eq!(SVGSpinnerProps::default().resolved_label(), "Loading...");
        assert_contains(
            &SimpleSVGSpinner(shared::SIZE_XS, "tiny").into_string(),
            "w-3 h-3",
        );
        assert_contains(
            &SimpleSVGSpinner(shared::SIZE_LG, "large").into_string(),
            "w-7 h-7",
        );
        assert_contains(
            &SimpleSVGSpinner("bad", "fallback").into_string(),
            "w-5 h-5",
        );

        let timer = SimpleTimer(120, 60, "timer-extra").into_string();
        assert_contains(&timer, "01:00");
        assert_contains(&timer, "timer-extra");
        assert_contains(&SessionTimer(3661, 3661, false).into_string(), "01:01:01");
        assert_contains(
            &SimpleNotificationBell(2, "bell-shortcut").into_string(),
            "bell-shortcut",
        );
        assert_contains(
            &SimpleAnimatedProgress(3, 6, "progress-shortcut").into_string(),
            "progress-shortcut",
        );

        let generating = GenerationStateIndicator(
            &GenerationStateIndicatorProps::default()
                .with_state("generating")
                .with_message("Working")
                .with_class("gen-extra")
                .with_attribute("data-testid", "gen"),
        )
        .into_string();
        assert_contains(&generating, "Generating");
        assert_contains(&generating, "Working");
        assert_contains(&generating, "gen-extra");
        let idle_no_message = GenerationStateIndicator(&GenerationStateIndicatorProps {
            show_spinner: false,
            ..Default::default()
        })
        .into_string();
        assert!(!idle_no_message.contains("Generating"));

        let processing = ProcessingProgress(&ProcessingProgressProps {
            message: "Silent".to_string(),
            percent: 150,
            show_label: false,
            class: "processing-extra".to_string(),
            attributes: BTreeMap::from([("data-testid".to_string(), "processing".to_string())]),
            ..Default::default()
        })
        .into_string();
        assert_contains(&processing, "width: 100%");
        assert_contains(&processing, "processing-extra");
        assert!(!processing.contains("Silent"));
        assert_eq!(iconOrSymbol("", "x", shared::SIZE_SM).into_string(), "");
        assert_contains(
            &SimpleStatFragment("1", "One").into_string(),
            "<span class=\"text-2xl font-bold text-foreground\">1</span>",
        );
        assert_eq!(
            StatFragmentProps {
                value_size: shared::SIZE_SM,
                ..Default::default()
            }
            .value_classes(),
            "text-sm font-semibold text-foreground"
        );
        assert_eq!(
            StatFragmentProps {
                value_size: shared::SIZE_MD,
                ..Default::default()
            }
            .value_classes(),
            "text-lg font-bold text-foreground"
        );
        assert_eq!(
            StatFragmentProps {
                value_size: shared::SIZE_LG,
                ..Default::default()
            }
            .value_classes(),
            "text-xl font-bold text-foreground"
        );
        let validation = ValidationMessage(&ValidationMessageProps {
            field: "name".to_string(),
            message: "Looks good".to_string(),
            is_error: false,
            class: "valid-extra".to_string(),
            attributes: BTreeMap::from([("data-testid".to_string(), "valid".to_string())]),
        })
        .into_string();
        assert_contains(&validation, "text-success valid-extra");
        assert_contains(&validation, r#"id="name-error""#);

        let plain_toast = ToastNotification(&ToastNotificationProps {
            message: "Info".to_string(),
            dismissible: false,
            duration: 250,
            class: "toast-extra".to_string(),
            attributes: BTreeMap::from([("data-testid".to_string(), "toast".to_string())]),
            ..Default::default()
        })
        .into_string();
        assert_contains(&plain_toast, "border-info bg-info-muted");
        assert_contains(&plain_toast, r#"data-duration="250""#);
        assert!(!plain_toast.contains("parentElement.remove"));
        assert_eq!(
            ToastNotificationProps {
                variant: shared::VARIANT_SUCCESS,
                ..Default::default()
            }
            .icon(),
            "✓"
        );
        assert_eq!(
            ToastNotificationProps {
                variant: shared::VARIANT_DESTRUCTIVE,
                ..Default::default()
            }
            .icon_classes(),
            "text-destructive"
        );
        assert_eq!(
            ToastNotificationProps {
                variant: shared::VARIANT_WARNING,
                ..Default::default()
            }
            .text_classes(),
            "text-warning-foreground"
        );

        assert_contains(
            &SimpleActivityItem("Ada", "joined").into_string(),
            "just now",
        );
        assert_contains(&TimerDisplay(-1, -2, "clock").into_string(), "00:00");
        assert_contains(&CountBadge(3, "", "count").into_string(), ">3</span>");
        assert_contains(
            &InlineNotification("success", "", "Saved").into_string(),
            r#"data-icon="check-circle""#,
        );
        assert_contains(
            &InlineNotification("warning", "Warn", "Careful").into_string(),
            r#"data-icon="alert-triangle""#,
        );
        assert_contains(
            &inlineNotificationIcon("unknown").into_string(),
            r#"data-icon="info""#,
        );

        let count_props = CountBadgeProps {
            count: 1,
            variant: shared::VARIANT_INFO,
            pulsing: true,
            class: "count-extra".to_string(),
            attributes: BTreeMap::from([("data-testid".to_string(), "count".to_string())]),
            ..Default::default()
        };
        assert!(count_props.should_show());
        assert!(count_props.build_classes().contains("animate-pulse"));
        assert_contains(
            &CountBadgeComponent(&count_props).into_string(),
            "count-extra",
        );

        let success_validation = FormLevelValidation(&FormLevelValidationProps {
            title: "Saved".to_string(),
            messages: vec!["Done".to_string()],
            is_error: false,
            dismissible: false,
            class: "form-extra".to_string(),
            attributes: BTreeMap::from([("data-testid".to_string(), "form".to_string())]),
        })
        .into_string();
        assert_contains(&success_validation, r#"role="status""#);
        assert_contains(&success_validation, "text-success");
        assert!(!success_validation.contains("currentTarget.closest"));

        for status in ["complete", "error", "idle"] {
            let props = ItemProgressProps {
                title: status.to_string(),
                status: status.to_string(),
                percent: 5,
                class: "item-progress-extra".to_string(),
                attributes: BTreeMap::from([("data-testid".to_string(), status.to_string())]),
                ..Default::default()
            };
            let html = ItemProgress(&props).into_string();
            assert_contains(&html, r#"data-testid=""#);
            assert_contains(&html, status);
        }
        assert!(
            ItemProgressProps {
                status: "uploading".to_string(),
                ..Default::default()
            }
            .is_processing()
        );
        assert_eq!(
            ItemProgressProps {
                status: "failed".to_string(),
                ..Default::default()
            }
            .status_icon(),
            "✕"
        );
        assert_eq!(
            ItemProgressProps {
                status: "unknown".to_string(),
                ..Default::default()
            }
            .status_icon(),
            "•"
        );
        assert_eq!(
            ItemProgressProps {
                status: "unknown".to_string(),
                ..Default::default()
            }
            .status_color(),
            "text-muted-foreground"
        );

        for status in ["complete", "error", "generating"] {
            let props = GenerationProgressProps {
                status: status.to_string(),
                message: "Message".to_string(),
                items_count: 2,
                items_label: "files".to_string(),
                class: "generation-extra".to_string(),
                attributes: BTreeMap::from([("data-testid".to_string(), status.to_string())]),
                ..Default::default()
            };
            let html = GenerationProgress(&props).into_string();
            assert_contains(&html, "Message");
            assert_contains(&html, "2 files");
            assert_contains(&html, "generation-extra");
        }
        assert_eq!(
            GenerationProgressProps {
                status: "pending".to_string(),
                ..Default::default()
            }
            .status_icon(),
            "…"
        );
        assert_contains(
            &SimpleGenerationProgress("", "complete").into_string(),
            "Complete",
        );

        let streaming = StreamingContent(&StreamingContentProps {
            icon: "spark".to_string(),
            content: "Done".to_string(),
            is_complete: true,
            show_cursor: true,
            class: "stream-extra".to_string(),
            attributes: BTreeMap::from([("data-testid".to_string(), "stream".to_string())]),
        })
        .into_string();
        assert_contains(&streaming, r#"data-icon="spark""#);
        assert!(!streaming.contains("inline-block w-2"));
        assert_contains(&streaming, "stream-extra");

        assert_eq!(
            ProgressCounter(&ProgressCounterProps {
                current: 3,
                total: 6,
                show_bar: false,
                ..Default::default()
            })
            .into_string(),
            "<div class=\"space-y-1\"><span class=\"text-sm text-muted-foreground\">3 / 6</span></div>"
        );

        let incoming = ChatBubble(&ChatBubbleProps {
            content: "Hi".to_string(),
            is_outgoing: false,
            class: "chat-extra".to_string(),
            attributes: BTreeMap::from([("data-testid".to_string(), "chat".to_string())]),
        })
        .into_string();
        assert_contains(&incoming, "flex justify-start");
        assert_contains(&incoming, "bg-muted text-foreground chat-extra");

        for trend in ["down", "flat"] {
            let props = StatCardProps {
                title: trend.to_string(),
                value: "1".to_string(),
                trend: trend.to_string(),
                trend_value: "1%".to_string(),
                ..Default::default()
            };
            let html = StatCard(&props).into_string();
            assert_contains(&html, props.trend_icon());
            assert_contains(&html, props.trend_color());
        }

        let pagination = Pagination(&PaginationProps {
            current_page: 1,
            total_pages: 2,
            base_url: "/p".to_string(),
            show_numbers: false,
            class: "pages-extra".to_string(),
            attributes: BTreeMap::from([("data-testid".to_string(), "pages".to_string())]),
            ..Default::default()
        })
        .into_string();
        assert_contains(&pagination, "pages-extra");
        assert!(!pagination.contains("?page=1\">1"));

        for (align, expected) in [("start", "items-start"), ("end", "items-end")] {
            assert!(
                SimpleStat(&SimpleStatProps {
                    value: "1".to_string(),
                    label: "One".to_string(),
                    align: align.to_string(),
                    size: shared::SIZE_SM,
                    ..Default::default()
                })
                .into_string()
                .contains(expected)
            );
        }
        assert_eq!(
            SimpleStatProps {
                size: shared::SIZE_MD,
                ..Default::default()
            }
            .value_classes(),
            "text-lg font-bold"
        );
        assert_eq!(
            SimpleStatProps {
                size: shared::SIZE_LG,
                ..Default::default()
            }
            .value_classes(),
            "text-xl font-bold"
        );

        assert_contains(
            &IconStatCard(&IconStatCardProps {
                label: "Label".to_string(),
                value: "2".to_string(),
                icon: "star".to_string(),
                style: String::new(),
                class: "icon-stat-extra".to_string(),
                attributes: BTreeMap::from([("data-testid".to_string(), "icon-stat".to_string())]),
            })
            .into_string(),
            "icon-stat-extra",
        );

        let avatar_user = UserListItem(&UserListItemProps {
            name: String::new(),
            href: String::new(),
            show_online: true,
            is_online: true,
            avatar_src: "/avatar.png".to_string(),
            value: "9".to_string(),
            value_label: "XP".to_string(),
            attributes: BTreeMap::from([("data-testid".to_string(), "user".to_string())]),
            ..Default::default()
        })
        .into_string();
        assert_contains(&avatar_user, r#"<img src="/avatar.png" alt=""#);
        assert_contains(&avatar_user, r#"data-testid="user""#);
        assert_eq!(
            UserListItemProps {
                name: String::new(),
                ..Default::default()
            }
            .initials(),
            "?"
        );

        let ranking = RankingCard(&RankingCardProps {
            rank: 3,
            name: "Ada".to_string(),
            subtitle: "Math".to_string(),
            value: 100,
            value_label: "XP".to_string(),
            secondary: "Top".to_string(),
            attributes: BTreeMap::from([("data-testid".to_string(), "rank".to_string())]),
            ..Default::default()
        })
        .into_string();
        assert_contains(&ranking, "Math");
        assert_contains(&ranking, "Top");
        assert_contains(&ranking, r#"data-testid="rank""#);

        assert_contains(
            &SectionCard(&SectionCardProps {
                title: "Title".to_string(),
                subtitle: "Subtitle".to_string(),
                icon: "book".to_string(),
                attributes: BTreeMap::from([("data-testid".to_string(), "section".to_string())]),
                ..Default::default()
            })
            .into_string(),
            "Subtitle",
        );
        assert_contains(
            &MasteryDisplay(&MasteryDisplayProps {
                percent: 110,
                label: String::new(),
                show_bar: true,
                attributes: BTreeMap::from([("data-testid".to_string(), "mastery".to_string())]),
                ..Default::default()
            })
            .into_string(),
            "width: 100%",
        );

        let row = ActivityRow(&ActivityRowProps {
            date: "Today".to_string(),
            stats: vec![ActivityRowStat {
                value: "10".to_string(),
                label: "XP".to_string(),
                variant: "success".to_string(),
            }],
            attributes: BTreeMap::from([("data-testid".to_string(), "row".to_string())]),
            ..Default::default()
        })
        .into_string();
        assert_contains(&row, "10");
        assert_contains(&row, "XP");

        for role in ["user", "system", "custom"] {
            let message = TranscriptMessage(&TranscriptMessageProps {
                role: role.to_string(),
                content: "Hi".to_string(),
                timestamp: "now".to_string(),
                timestamp_iso: "2026-04-30T00:00:00Z".to_string(),
                ..Default::default()
            })
            .into_string();
            assert_contains(&message, "now");
            assert_contains(&message, "Hi");
        }

        let link = ListItemLinkCard(&ListItemLinkCardProps {
            href: "/lesson".to_string(),
            title: "Lesson".to_string(),
            status: "active".to_string(),
            timestamp: "today".to_string(),
            show_arrow: true,
            stats: vec![IconStat {
                icon: "book".to_string(),
                value: "2".to_string(),
                label: "docs".to_string(),
            }],
            ..Default::default()
        })
        .into_string();
        assert_contains(&link, r#"data-icon="book""#);
        assert_contains(&link, "2 docs");
        assert_contains(&link, "→");

        assert_contains(
            &StatIconRow(&StatIconRowProps {
                stats: vec![IconStat {
                    icon: "star".to_string(),
                    value: "5".to_string(),
                    label: "stars".to_string(),
                }],
                ..Default::default()
            })
            .into_string(),
            r#"data-icon="star""#,
        );
        assert_contains(
            &StatusBadge(&StatusBadgeProps {
                status: "Nope".to_string(),
                status_type: "destructive".to_string(),
                class: "status-badge-extra".to_string(),
                attributes: BTreeMap::from([(
                    "data-testid".to_string(),
                    "status-badge".to_string(),
                )]),
            })
            .into_string(),
            "status-badge-extra",
        );

        let disabled_item = ItemInfoCard(&ItemInfoCardProps {
            icon: "gift".to_string(),
            title: "Reward".to_string(),
            badge: "New".to_string(),
            is_disabled: true,
            metadata: vec![ItemInfoMeta {
                icon: "clock".to_string(),
                label: "When".to_string(),
                value: "Today".to_string(),
                variant: "warning".to_string(),
            }],
            class: "info-extra".to_string(),
            attributes: BTreeMap::from([("data-testid".to_string(), "info".to_string())]),
            ..Default::default()
        })
        .into_string();
        assert_contains(&disabled_item, "opacity-60 grayscale");
        assert_contains(&disabled_item, "New");
        assert_contains(&disabled_item, "Today");
    }
}
